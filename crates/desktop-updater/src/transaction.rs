//! Recoverable application-file replacement; callers retain the installation lock.
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
const MAX_JOURNAL_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum Recovery {
    /// Preparation never became durable; installation files were not changed.
    /// Under the installation lock, the caller may discard this protected stage
    /// and prepare a fresh one. Do not reuse its potentially partial backups.
    Unprepared,
    Confirmed,
    RolledBack,
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Applied,
    Confirmed,
    RolledBack,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    new_hash: String,
    old_hash: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: String,
    phase: Phase,
    entries: Vec<Entry>,
}

pub struct Transaction {
    installation: PathBuf,
    directory: PathBuf,
    journal: Journal,
}
impl Transaction {
    /// `directory/new` must contain the verified archive in protected storage.
    /// The broker supplies both roots from its own registration, never a client.
    pub fn prepare(installation: &Path, directory: &Path, version: &str) -> anyhow::Result<Self> {
        ensure!(version.len() <= 128, "更新版本过长");
        semver::Version::parse(version)?;
        no_link(installation)?;
        no_link(directory)?;
        let installation = installation.canonicalize()?;
        let directory = directory.canonicalize()?;
        ensure!(
            !directory.starts_with(&installation) && !installation.starts_with(&directory),
            "安装目录与更新状态目录不能嵌套"
        );
        ensure!(
            !directory.join("journal.json").exists() && !directory.join("old").exists(),
            "更新事务已存在"
        );
        no_link(&directory.join("new"))?;
        let mut paths = Vec::new();
        collect(&directory.join("new"), "", &mut paths)?;
        ensure!(!paths.is_empty(), "更新包没有文件");
        paths.sort();
        let mut names = std::collections::BTreeSet::new();
        let mut entries = Vec::new();
        for path in paths {
            super::safe_entry_path(&path)?;
            ensure!(names.insert(path.to_lowercase()), "更新文件名重复");
            check_target(&installation, &path)?;
            let target = installation.join(&path);
            let old_hash = if target.try_exists()? {
                let digest = hash(&target)?;
                copy(&target, &directory.join("old").join(&path), &directory)?;
                Some(digest)
            } else {
                None
            };
            entries.push(Entry {
                new_hash: hash(&directory.join("new").join(&path))?,
                path,
                old_hash,
            });
        }
        let transaction = Self {
            installation,
            directory,
            journal: Journal {
                version: version.into(),
                phase: Phase::Prepared,
                entries,
            },
        };
        transaction.save()?;
        Ok(transaction)
    }
    pub fn apply(&mut self) -> anyhow::Result<()> {
        ensure!(
            self.journal.phase == Phase::Prepared,
            "更新事务不能重复应用"
        );
        for entry in &self.journal.entries {
            check_target(&self.installation, &entry.path)?;
            ensure!(
                hash(&self.directory.join("new").join(&entry.path))? == entry.new_hash,
                "暂存更新文件已改变"
            );
        }
        // Prepared is already durable. Any failure below is recovered from the
        // saved originals; startup treats unconfirmed Applied the same way.
        for entry in &self.journal.entries {
            copy(
                &self.directory.join("new").join(&entry.path),
                &self.installation.join(&entry.path),
                &self.installation,
            )?;
        }
        self.transition(Phase::Applied)
    }
    pub fn confirm(&mut self) -> anyhow::Result<()> {
        ensure!(
            self.journal.phase == Phase::Applied,
            "不能确认尚未应用的更新"
        );
        self.transition(Phase::Confirmed)
    }
    pub fn recover(installation: &Path, directory: &Path) -> anyhow::Result<Recovery> {
        no_link(installation)?;
        no_link(directory)?;
        let installation = installation.canonicalize()?;
        let directory = directory.canonicalize()?;
        ensure!(
            !directory.starts_with(&installation) && !installation.starts_with(&directory),
            "更新恢复目录无效"
        );
        let mut bytes = Vec::new();
        let path = directory.join("journal.json");
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Recovery::Unprepared);
            }
            Err(error) => return Err(error.into()),
            Ok(_) => {}
        }
        no_link(&path)?;
        fs::File::open(path)?
            .take(MAX_JOURNAL_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_JOURNAL_BYTES, "更新日志过大");
        let journal: Journal = serde_json::from_slice(&bytes)?;
        let mut transaction = Self {
            installation,
            directory,
            journal,
        };
        if transaction.journal.phase == Phase::Confirmed {
            return Ok(Recovery::Confirmed);
        }
        let restore_originals = transaction.journal.phase != Phase::RolledBack;
        ensure!(
            transaction.journal.entries.len() <= 16384,
            "更新日志文件数量无效"
        );
        // Check every backup before changing even one installed file.
        for entry in &transaction.journal.entries {
            if !restore_originals && entry.old_hash.is_some() {
                continue;
            }
            super::safe_entry_path(&entry.path)?;
            check_target(&transaction.installation, &entry.path)?;
            if let Some(expected) = &entry.old_hash {
                check_target(&transaction.directory, &format!("old/{}", entry.path))?;
                ensure!(
                    hash(&transaction.directory.join("old").join(&entry.path))? == *expected,
                    "旧版备份损坏，停止自动恢复"
                );
            } else {
                let target = transaction.installation.join(&entry.path);
                if target.try_exists()? {
                    ensure!(
                        hash(&target)? == entry.new_hash,
                        "新增文件被修改，停止自动删除"
                    );
                }
            }
        }
        for entry in &transaction.journal.entries {
            let target = transaction.installation.join(&entry.path);
            if entry.old_hash.is_some() {
                if restore_originals {
                    copy(
                        &transaction.directory.join("old").join(&entry.path),
                        &target,
                        &transaction.installation,
                    )?;
                }
            } else if target.try_exists()? {
                ensure!(
                    hash(&target)? == entry.new_hash,
                    "新增文件被修改，停止自动删除"
                );
                fs::remove_file(&target)?;
                sync_parents(target.parent().unwrap(), &transaction.installation)?;
            }
        }
        // Reconcile deletions even for RolledBack: Windows does not provide
        // our Unix directory-fsync barrier for remove_file. A surviving stale
        // file is removed on recovery before the caller resumes the desktop.
        transaction.transition(Phase::RolledBack)?;
        Ok(Recovery::RolledBack)
    }
    fn transition(&mut self, phase: Phase) -> anyhow::Result<()> {
        let previous = std::mem::replace(&mut self.journal.phase, phase);
        if let Err(error) = self.save() {
            // Failed persistence must never unlock the next lifecycle step.
            // Retrying writes the same intended state, including when the
            // replacement succeeded but its subsequent directory sync failed.
            self.journal.phase = previous;
            return Err(error);
        }
        Ok(())
    }
    fn save(&self) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(&self.journal)?;
        // Reserve the longest phase spelling so a prepared journal cannot
        // exceed the recovery limit on a later state transition.
        let largest = bytes.len() - serde_json::to_vec(&self.journal.phase)?.len()
            + serde_json::to_vec(&Phase::RolledBack)?.len();
        ensure!(largest <= MAX_JOURNAL_BYTES, "更新日志过大");
        atomic_write(&self.directory.join("journal.json"), bytes.as_slice())?;
        sync_parents(&self.directory, &self.directory)
    }
}

fn collect(directory: &Path, prefix: &str, paths: &mut Vec<String>) -> anyhow::Result<()> {
    for child in fs::read_dir(directory)? {
        let child = child?;
        let metadata = no_link(&child.path())?;
        let component = child
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("更新路径编码无效"))?;
        let path = if prefix.is_empty() {
            component
        } else {
            format!("{prefix}/{component}")
        };
        super::safe_entry_path(&path)?;
        if metadata.is_dir() {
            collect(&child.path(), &path, paths)?;
        } else {
            ensure!(metadata.is_file(), "更新文件类型无效");
            paths.push(path);
            ensure!(paths.len() <= 16384, "更新文件数量过多");
        }
    }
    Ok(())
}
fn no_link(path: &Path) -> anyhow::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(!metadata.file_type().is_symlink(), "更新路径不能包含链接");
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "更新路径不能包含重解析点"
        );
    }
    Ok(metadata)
}
fn check_target(root: &Path, relative: &str) -> anyhow::Result<()> {
    super::safe_entry_path(relative)?;
    let mut path = root.to_path_buf();
    no_link(&path)?;
    for component in relative.split('/') {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                no_link(&path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
fn hash(path: &Path) -> anyhow::Result<String> {
    ensure!(no_link(path)?.is_file(), "更新对象不是普通文件");
    let mut source = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut size = 0u64;
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        ensure!(size <= super::MAX_PACKAGE_BYTES, "更新文件过大");
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn copy(source: &Path, target: &Path, boundary: &Path) -> anyhow::Result<()> {
    let relative = target
        .strip_prefix(boundary)?
        .components()
        .map(|part| part.as_os_str().to_str().context("更新路径编码无效"))
        .collect::<anyhow::Result<Vec<_>>>()?
        .join("/");
    check_target(boundary, &relative)?;
    fs::create_dir_all(target.parent().context("更新文件没有父目录")?)?;
    atomic_write(target, fs::File::open(source)?)?;
    sync_parents(target.parent().unwrap(), boundary)
}
fn atomic_write(target: &Path, mut source: impl Read) -> anyhow::Result<()> {
    let mut temporary =
        tempfile::NamedTempFile::new_in(target.parent().context("更新文件没有父目录")?)?;
    std::io::copy(&mut source, &mut temporary)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    let temporary = temporary.into_temp_path();
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        ensure!(
            unsafe {
                MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            } != 0,
            "更新文件替换失败：{}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(not(windows))]
    fs::rename(&temporary, target)?;
    Ok(())
}
fn sync_parents(path: &Path, boundary: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    for parent in path
        .ancestors()
        .take_while(|parent| parent.starts_with(boundary))
    {
        fs::File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = (path, boundary);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("installed");
        let transaction = temp.path().join("transaction");
        fs::create_dir(&app).unwrap();
        fs::create_dir_all(transaction.join("new/resources")).unwrap();
        fs::write(app.join("app.exe"), b"old").unwrap();
        fs::write(app.join("user.config"), b"preserve").unwrap();
        fs::write(transaction.join("new/app.exe"), b"new").unwrap();
        fs::write(transaction.join("new/resources/worker.exe"), b"worker").unwrap();
        (temp, app, transaction)
    }
    #[test]
    fn interrupted_apply_and_failed_health_restore_original_files() {
        for partial in [true, false] {
            let (_temp, app, directory) = fixture();
            let mut transaction = Transaction::prepare(&app, &directory, "2.0.0").unwrap();
            assert_eq!(fs::read(app.join("app.exe")).unwrap(), b"old");
            if partial {
                fs::write(app.join("app.exe"), b"new").unwrap();
            } else {
                transaction.apply().unwrap();
                assert!(app.join("resources/worker.exe").exists());
            }
            drop(transaction);
            Transaction::recover(&app, &directory).unwrap();
            assert_eq!(fs::read(app.join("app.exe")).unwrap(), b"old");
            assert_eq!(fs::read(app.join("user.config")).unwrap(), b"preserve");
            assert!(!app.join("resources/worker.exe").exists());
            Transaction::recover(&app, &directory).unwrap();
        }
    }
    #[test]
    fn confirmed_update_survives_recovery_and_cannot_confirm_without_apply() {
        let (_temp, app, directory) = fixture();
        let mut transaction = Transaction::prepare(&app, &directory, "2.0.0").unwrap();
        assert!(transaction.confirm().is_err());
        transaction.apply().unwrap();
        transaction.confirm().unwrap();
        Transaction::recover(&app, &directory).unwrap();
        assert_eq!(fs::read(app.join("app.exe")).unwrap(), b"new");
        assert_eq!(
            fs::read(app.join("resources/worker.exe")).unwrap(),
            b"worker"
        );
    }
    #[test]
    fn corrupt_backup_is_rejected_before_restore_changes_any_file() {
        let (_temp, app, directory) = fixture();
        let mut transaction = Transaction::prepare(&app, &directory, "2.0.0").unwrap();
        transaction.apply().unwrap();
        fs::write(directory.join("old/app.exe"), b"corrupt").unwrap();
        assert!(Transaction::recover(&app, &directory).is_err());
        assert_eq!(fs::read(app.join("app.exe")).unwrap(), b"new");
        assert!(app.join("resources/worker.exe").exists());
    }

    #[test]
    fn failed_journal_write_allows_retry_without_false_confirmation() {
        for during_confirm in [false, true] {
            let (_temp, app, directory) = fixture();
            let mut transaction = Transaction::prepare(&app, &directory, "2.0.0").unwrap();
            if during_confirm {
                transaction.apply().unwrap();
            }
            let journal = directory.join("journal.json");
            fs::rename(&journal, directory.join("saved-journal.json")).unwrap();
            fs::create_dir(&journal).unwrap();
            if during_confirm {
                assert!(transaction.confirm().is_err());
            } else {
                assert!(transaction.apply().is_err());
            }
            fs::remove_dir(&journal).unwrap();
            fs::rename(directory.join("saved-journal.json"), &journal).unwrap();
            if !during_confirm {
                assert!(
                    transaction.confirm().is_err(),
                    "failed apply must not become confirmable"
                );
                transaction.apply().unwrap();
            }
            transaction.confirm().unwrap();
            Transaction::recover(&app, &directory).unwrap();
            assert_eq!(fs::read(app.join("app.exe")).unwrap(), b"new");
        }
    }

    #[test]
    fn changed_new_file_blocks_recovery_before_any_replacement() {
        let (_temp, app, directory) = fixture();
        let mut transaction = Transaction::prepare(&app, &directory, "2.0.0").unwrap();
        transaction.apply().unwrap();
        fs::write(app.join("resources/worker.exe"), b"external change").unwrap();
        assert!(Transaction::recover(&app, &directory).is_err());
        assert_eq!(fs::read(app.join("app.exe")).unwrap(), b"new");
        assert_eq!(
            fs::read(app.join("resources/worker.exe")).unwrap(),
            b"external change"
        );
    }

    #[test]
    fn journal_too_large_to_recover_is_rejected_before_persistence() {
        let (_temp, app, directory) = fixture();
        let mut transaction = Transaction::prepare(&app, &directory, "2.0.0").unwrap();
        let original = fs::read(directory.join("journal.json")).unwrap();
        transaction.journal.entries = (0..16384)
            .map(|i| Entry {
                path: format!("{}/{i}.exe", "文".repeat(220)),
                new_hash: "a".repeat(64),
                old_hash: None,
            })
            .collect();
        assert!(serde_json::to_vec(&transaction.journal).unwrap().len() > 8 * 1024 * 1024);
        assert!(transaction.save().is_err());
        assert_eq!(fs::read(directory.join("journal.json")).unwrap(), original);
        assert_eq!(fs::read(app.join("app.exe")).unwrap(), b"old");
    }

    #[test]
    fn completed_rollback_reconciles_reappearing_new_file() {
        let (_temp, app, directory) = fixture();
        let mut transaction = Transaction::prepare(&app, &directory, "2.0.0").unwrap();
        transaction.apply().unwrap();
        Transaction::recover(&app, &directory).unwrap();
        // Model a persisted terminal journal but a lost deletion directory update.
        fs::write(app.join("resources/worker.exe"), b"worker").unwrap();
        fs::write(app.join("app.exe"), b"old app subsequently touched").unwrap();
        Transaction::recover(&app, &directory).unwrap();
        assert!(!app.join("resources/worker.exe").exists());
        assert_eq!(
            fs::read(app.join("app.exe")).unwrap(),
            b"old app subsequently touched"
        );
    }

    #[test]
    fn interrupted_preparation_reports_unprepared_without_changing_installation() {
        let (_temp, app, directory) = fixture();
        fs::create_dir(directory.join("old")).unwrap();
        fs::write(directory.join("old/app.exe"), b"partial backup").unwrap();
        let outcome = Transaction::recover(&app, &directory).unwrap();
        assert_eq!(outcome, Recovery::Unprepared);
        assert_eq!(fs::read(app.join("app.exe")).unwrap(), b"old");
        assert_eq!(
            fs::read(directory.join("old/app.exe")).unwrap(),
            b"partial backup"
        );
    }

    #[cfg(windows)]
    #[test]
    fn locked_file_partial_apply_is_recoverable_after_handle_closes() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_temp, app, directory) = fixture();
        fs::create_dir_all(app.join("resources")).unwrap();
        fs::write(app.join("resources/worker.exe"), b"old worker").unwrap();
        let mut transaction = Transaction::prepare(&app, &directory, "2.0.0").unwrap();
        let handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(app.join("resources/worker.exe"))
            .unwrap();
        assert!(transaction.apply().is_err());
        assert!(transaction.confirm().is_err());
        assert_eq!(fs::read(app.join("app.exe")).unwrap(), b"new");
        drop(handle);
        Transaction::recover(&app, &directory).unwrap();
        assert_eq!(fs::read(app.join("app.exe")).unwrap(), b"old");
        assert_eq!(
            fs::read(app.join("resources/worker.exe")).unwrap(),
            b"old worker"
        );
    }
}
