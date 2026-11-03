use super::*;
use std::process::Command;

fn run(path: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command.arg("-C").arg(path).args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{:?}: {}",
        args,
        text(&result.stderr)
    );
    text(&result.stdout).trim().to_owned()
}

fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    run(directory.path(), &["init", "-b", "main"]);
    run(directory.path(), &["config", "user.name", "Example"]);
    run(
        directory.path(),
        &["config", "user.email", "example@example.invalid"],
    );
    run(directory.path(), &["config", "commit.gpgsign", "false"]);
    directory
}

fn commit(path: &Path, message: &str, seconds: i64) -> String {
    run(path, &["add", "--all"]);
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(path)
        .args(["commit", "-m", message])
        .env("GIT_AUTHOR_DATE", format!("{seconds} +0000"))
        .env("GIT_COMMITTER_DATE", format!("{seconds} +0000"));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let result = command.output().unwrap();
    assert!(result.status.success(), "{}", text(&result.stderr));
    run(path, &["rev-parse", "HEAD"])
}

async fn artifact(path: &Path, cutoff: i64, known: &BTreeSet<String>) -> Artifact {
    collect(
        path,
        "task".into(),
        7,
        "group/repository".into(),
        cutoff,
        known,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn all_branches_merge_binary_rename_window_and_cached_facts() {
    let repository = repository();
    let path = repository.path();
    std::fs::write(path.join("root.txt"), "one\n").unwrap();
    std::fs::write(path.join(".hidden"), "hidden\n").unwrap();
    std::fs::write(path.join("blob.bin"), [0, 1, 2]).unwrap();
    let root = commit(path, "initial", 1_700_000_300);
    run(path, &["branch", "side"]);
    std::fs::remove_file(path.join("root.txt")).unwrap();
    std::fs::write(path.join("root.rs"), "one\ntwo\n").unwrap();
    std::fs::write(path.join("blob.bin"), [0, 3, 4]).unwrap();
    let middle = commit(path, "rename and binary", 1_700_000_100);
    run(path, &["checkout", "side"]);
    std::fs::write(path.join("side.rs"), "side\n").unwrap();
    let side = commit(path, "side", 1_700_000_400);
    run(path, &["checkout", "main"]);
    run(path, &["merge", "--no-ff", "--no-commit", "side"]);
    let merge = commit(path, "merge", 1_700_000_500);
    run(path, &["update-ref", "refs/remotes/origin/main", &merge]);
    run(
        path,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    let full = artifact(path, 0, &BTreeSet::new()).await;
    assert_eq!(full.refs.len(), 2);
    assert_eq!(full.ref_commits["main"].len(), 4);
    assert_eq!(full.ref_commits["side"].len(), 2);
    let facts: BTreeMap<_, _> = full.commits.iter().map(|c| (c.sha.as_str(), c)).collect();
    assert_eq!(
        (
            facts[root.as_str()].additions,
            facts[root.as_str()].deletions
        ),
        (0, 0)
    );
    assert_eq!(
        facts[root.as_str()].language_stats,
        BTreeMap::from([
            (".hidden".into(), 1),
            (".txt".into(), 1),
            (".bin".into(), 1)
        ])
    );
    assert_eq!(
        (
            facts[middle.as_str()].additions,
            facts[middle.as_str()].deletions
        ),
        (2, 1)
    );
    assert_eq!(
        facts[middle.as_str()].language_stats,
        BTreeMap::from([(".txt".into(), 1), (".rs".into(), 1), (".bin".into(), 1)])
    );
    assert_eq!(
        (
            facts[merge.as_str()].additions,
            facts[merge.as_str()].deletions
        ),
        (3, 1)
    );
    assert_eq!(
        facts[merge.as_str()].language_stats,
        BTreeMap::from([(".rs".into(), 1)])
    );
    assert_eq!(facts[merge.as_str()].parents.len(), 2);
    assert!(facts[side.as_str()].complete);
    let window = artifact(path, 1_700_000_300_000, &BTreeSet::new()).await;
    assert!(window.ref_commits["main"].contains(&root)); // Reach across the older child.
    assert!(!window.ref_commits["main"].contains(&middle));
    assert_eq!(window.commits.len(), 4); // Full proof graph still exists.
    let old = window.commits.iter().find(|c| c.sha == middle).unwrap();
    assert!(!old.complete);
    assert_eq!((old.additions, old.deletions), (0, 0));
    assert!(old.language_stats.is_empty());
    let window_known = window.commits.iter().map(|c| c.sha.clone()).collect();
    let cached_window = artifact(path, 1_700_000_300_000, &window_known).await;
    assert!(cached_window.commits.is_empty());
    assert_eq!(cached_window.ref_commits, window.ref_commits);
    let known = full.commits.iter().map(|c| c.sha.clone()).collect();
    let cached = artifact(path, 0, &known).await;
    assert!(cached.commits.is_empty());
    assert_eq!(cached.ref_commits, full.ref_commits);
    assert_eq!(cached.refs, full.refs);
}

#[tokio::test]
async fn empty_repository_and_invalid_repository() {
    let repository = repository();
    let empty = artifact(repository.path(), 0, &BTreeSet::new()).await;
    assert!(empty.complete && empty.refs.is_empty() && empty.commits.is_empty());
    let missing = repository.path().join("does-not-exist");
    assert!(
        collect(
            &missing,
            "task".into(),
            1,
            "repo".into(),
            0,
            &BTreeSet::new()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn raw_negative_timestamp_keeps_newer_ancestor_and_conflicting_refs_fail() {
    let repository = repository();
    let path = repository.path();
    std::fs::write(path.join("file"), "data\n").unwrap();
    let root = commit(path, "root", 1_700_000_300);
    let tree = run(path, &["rev-parse", "HEAD^{tree}"]);
    let raw = format!(
        "tree {tree}\nparent {root}\nauthor Pre Epoch <pre@example.invalid> -20 +0000\ncommitter Pre Epoch <pre@example.invalid> -10 +0000\n\nnegative\n"
    );
    let sha = text(
        &git(
            path,
            &[
                "hash-object",
                "--literally",
                "-w",
                "-t",
                "commit",
                "--stdin",
            ],
            Some(raw.into_bytes()),
        )
        .await
        .unwrap(),
    )
    .trim()
    .to_owned();
    run(path, &["update-ref", "refs/heads/main", &sha]);
    let window = artifact(path, 1_700_000_300_000, &BTreeSet::new()).await;
    assert_eq!(window.ref_commits["main"], vec![root.clone()]);
    let negative = window.commits.iter().find(|c| c.sha == sha).unwrap();
    assert_eq!(
        (negative.author_date, negative.committer_date),
        (-20_000, -10_000)
    );
    assert!(!negative.complete);
    run(path, &["update-ref", "refs/remotes/origin/main", &root]);
    assert!(
        collect(path, "task".into(), 1, "repo".into(), 0, &BTreeSet::new())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn declared_legacy_encoding_preserves_names_and_message() {
    let repository = repository();
    let path = repository.path();
    std::fs::write(path.join("file"), "data\n").unwrap();
    commit(path, "root", 1_700_000_300);
    let tree = run(path, &["rev-parse", "HEAD^{tree}"]);
    let mut raw = format!("tree {tree}\nauthor Andr").into_bytes();
    raw.extend_from_slice(b"\xe9 <legacy@example.invalid> 1700000300 +0000\ncommitter Example <legacy@example.invalid> 1700000300 +0000\nencoding ISO-8859-1\n\ncaf\xe9\n");
    let sha = text(
        &git(
            path,
            &["hash-object", "-w", "-t", "commit", "--stdin"],
            Some(raw),
        )
        .await
        .unwrap(),
    )
    .trim()
    .to_owned();
    run(path, &["update-ref", "refs/heads/main", &sha]);
    let result = artifact(path, 0, &BTreeSet::new()).await;
    assert_eq!(result.commits.len(), 1);
    assert_eq!(result.commits[0].author_name, "André");
    assert_eq!(result.commits[0].message, "café\n");
}
