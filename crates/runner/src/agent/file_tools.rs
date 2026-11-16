//! File-production tools the execution user already has, so an Agent asked for
//! a document, sheet, deck, chart or video can pick a route that works here.
//! Ferry only looks; it never installs anything.
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

/// How long one tool may take to answer `--version`.
const TOOL_LIMIT: Duration = Duration::from_secs(6);
/// Tools rarely change; a fresh report at most this often.
const CACHE_FOR: Duration = Duration::from_secs(600);
/// Python modules worth knowing about for file production.
const MODULES: &[(&str, &str)] = &[
    ("docx", "python-docx"),
    ("openpyxl", "openpyxl"),
    ("pptx", "python-pptx"),
    ("reportlab", "reportlab"),
    ("weasyprint", "weasyprint"),
    ("matplotlib", "matplotlib"),
    ("PIL", "Pillow"),
    ("pandas", "pandas"),
    ("fitz", "PyMuPDF"),
    ("markdown", "markdown"),
];
const PYTHON_SCRIPT: &str = "import importlib.util as u,sys\nprint(sys.version.split()[0])\nfor m in sys.argv[1:]:\n    print(m if u.find_spec(m) else '')";

static CACHE: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);

/// Short descriptions such as "python 3.12.1 (python-docx, openpyxl)".
pub(crate) async fn report() -> Vec<String> {
    if let Some((at, tools)) = CACHE.lock().unwrap_or_else(|p| p.into_inner()).clone() {
        if at.elapsed() < CACHE_FOR {
            return tools;
        }
    }
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    let tools = collect(super::paths::search(), home.as_deref()).await;
    *CACHE.lock().unwrap_or_else(|p| p.into_inner()) = Some((Instant::now(), tools.clone()));
    tools
}

pub(super) async fn collect(search: &[PathBuf], home: Option<&Path>) -> Vec<String> {
    let find = |names: &[&str]| names.iter().find_map(|name| locate(search, name));
    let python = find(&["python3", "python"]);
    let (python, node, ffmpeg, pandoc, typst) = tokio::join!(
        async {
            match &python {
                Some(path) => python_report(path).await,
                None => None,
            }
        },
        version(find(&["node"]), &["--version"], "node"),
        version(find(&["ffmpeg"]), &["-version"], "ffmpeg"),
        version(find(&["pandoc"]), &["--version"], "pandoc"),
        version(find(&["typst"]), &["--version"], "typst"),
    );
    let mut tools: Vec<String> = [python, node, ffmpeg, pandoc, typst]
        .into_iter()
        .flatten()
        .collect();
    // Office's own binary has no console output on Windows: presence is enough.
    if find(&["soffice", "libreoffice"]).is_some() || office_installed() {
        tools.push("libreoffice".into());
    }
    if let Some(home) = home {
        for (label, directory) in [
            ("claude skills", home.join(".claude").join("skills")),
            ("codex skills", home.join(".codex").join("skills")),
            ("agent skills", home.join(".agents").join("skills")),
        ] {
            let names = skills(&directory);
            if !names.is_empty() {
                tools.push(format!("{label}: {}", names.join(", ")));
            }
        }
    }
    tools
}

/// An executable named `name` on the search path (Windows: `.exe` only, since
/// `.cmd` shims need a shell).
pub(super) fn locate(search: &[PathBuf], name: &str) -> Option<PathBuf> {
    search
        .iter()
        .filter(|directory| directory.is_absolute())
        .map(|directory| {
            directory.join(if cfg!(windows) {
                format!("{name}.exe")
            } else {
                name.to_owned()
            })
        })
        // The Microsoft Store placeholder opens the Store instead of running.
        .find(|path| {
            path.is_file()
                && !path
                    .to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("windowsapps")
        })
}

fn office_installed() -> bool {
    let candidates: &[&str] = if cfg!(windows) {
        &[
            r"C:\Program Files\LibreOffice\program\soffice.exe",
            r"C:\Program Files (x86)\LibreOffice\program\soffice.exe",
        ]
    } else if cfg!(target_os = "macos") {
        &["/Applications/LibreOffice.app/Contents/MacOS/soffice"]
    } else {
        &[]
    };
    candidates.iter().any(|path| Path::new(path).is_file())
}

/// Skill directory names (each skill is a folder), sorted, at most 30.
pub(super) fn skills(directory: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.') && name.len() <= 64)
        .collect();
    names.sort();
    names.truncate(30);
    names
}

async fn output(executable: &Path, args: &[&str]) -> Option<String> {
    let mut command = tokio::process::Command::new(executable);
    command
        .args(args)
        .env("PATH", super::paths::child_path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let result = tokio::time::timeout(TOOL_LIMIT, command.output())
        .await
        .ok()?
        .ok()?;
    let mut text = String::from_utf8_lossy(&result.stdout).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&result.stderr).into_owned();
    }
    result.status.success().then_some(text)
}

/// "name <version>" from the first output line's first version-like word.
async fn version(executable: Option<PathBuf>, args: &[&str], name: &str) -> Option<String> {
    let text = output(&executable?, args).await?;
    Some(match version_word(&text) {
        Some(version) => format!("{name} {version}"),
        None => name.to_owned(),
    })
}

pub(super) fn version_word(text: &str) -> Option<String> {
    let line = text.lines().find(|line| !line.trim().is_empty())?;
    line.split_whitespace()
        .map(|word| word.trim_start_matches(['v', 'V']))
        .find(|word| {
            word.chars().next().is_some_and(|c| c.is_ascii_digit())
                && word.contains('.')
                && word.len() <= 32
        })
        .map(|word| word.trim_end_matches([',', ';']).to_owned())
}

async fn python_report(executable: &Path) -> Option<String> {
    let mut args = vec!["-c", PYTHON_SCRIPT];
    args.extend(MODULES.iter().map(|(module, _)| *module));
    python_line(&output(executable, &args).await?)
}

/// "python 3.12.1 (python-docx, matplotlib)" from the probe script's output.
pub(super) fn python_line(text: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim);
    let version = lines
        .next()
        .filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))?;
    let found: Vec<&str> = lines
        .filter(|line| !line.is_empty())
        .filter_map(|module| {
            MODULES
                .iter()
                .find(|(name, _)| name == &module)
                .map(|(_, package)| *package)
        })
        .collect();
    Some(if found.is_empty() {
        format!("python {version}")
    } else {
        format!("python {version} ({})", found.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_python_modules_are_summarised() {
        assert_eq!(version_word("v22.11.0\n").as_deref(), Some("22.11.0"));
        assert_eq!(
            version_word("ffmpeg version 6.1.1 Copyright (c) 2000-2023").as_deref(),
            Some("6.1.1")
        );
        assert_eq!(
            version_word("pandoc 3.1.11\nFeatures: +server").as_deref(),
            Some("3.1.11")
        );
        assert_eq!(version_word("no version here"), None);
        assert_eq!(
            python_line("3.12.1\ndocx\n\npptx\nPIL\n").as_deref(),
            Some("python 3.12.1 (python-docx, python-pptx, Pillow)")
        );
        assert_eq!(
            python_line("3.11.2\n\n\n").as_deref(),
            Some("python 3.11.2")
        );
        // A Store placeholder or broken interpreter answers something else.
        assert_eq!(
            python_line("Python was not found; run without arguments"),
            None
        );
    }

    #[test]
    fn skills_and_tools_are_found_only_where_they_exist() {
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join(".claude").join("skills");
        std::fs::create_dir_all(skills_dir.join("pptx")).unwrap();
        std::fs::create_dir_all(skills_dir.join("docx")).unwrap();
        std::fs::create_dir_all(skills_dir.join(".hidden")).unwrap();
        std::fs::write(skills_dir.join("README.md"), "not a skill").unwrap();
        assert_eq!(skills(&skills_dir), ["docx", "pptx"]);
        assert!(skills(&dir.path().join("missing")).is_empty());
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let name = if cfg!(windows) {
            "pandoc.exe"
        } else {
            "pandoc"
        };
        std::fs::write(bin.join(name), "").unwrap();
        assert_eq!(locate(&[bin.clone()], "pandoc"), Some(bin.join(name)));
        assert_eq!(locate(&[bin.clone()], "ffmpeg"), None);
        // Relative entries never search the working directory.
        assert_eq!(locate(&[PathBuf::from("bin")], "pandoc"), None);
    }

    #[tokio::test]
    async fn skill_folders_are_reported_without_any_tools() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".codex").join("skills").join("xlsx")).unwrap();
        let tools = collect(&[], Some(dir.path())).await;
        assert!(
            tools.contains(&"codex skills: xlsx".to_owned()),
            "{tools:?}"
        );
    }
}
