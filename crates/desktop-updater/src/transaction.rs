//! Recoverable application-file replacement; callers retain the installation lock.
use std::path::Path;

pub struct Transaction;
impl Transaction {
    pub fn prepare(_installation: &Path, _directory: &Path, _version: &str) -> anyhow::Result<Self> {
        anyhow::bail!("update transaction is not implemented")
    }
    pub fn apply(&mut self) -> anyhow::Result<()> { anyhow::bail!("update transaction is not implemented") }
    pub fn confirm(&mut self) -> anyhow::Result<()> { anyhow::bail!("update transaction is not implemented") }
    pub fn recover(_installation: &Path, _directory: &Path) -> anyhow::Result<()> { anyhow::bail!("update transaction is not implemented") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn fixture() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let temp=tempfile::tempdir().unwrap();
        let app=temp.path().join("installed");let transaction=temp.path().join("transaction");
        fs::create_dir(&app).unwrap();fs::create_dir_all(transaction.join("new/resources")).unwrap();
        fs::write(app.join("app.exe"),b"old").unwrap();
        fs::write(app.join("user.config"),b"preserve").unwrap();
        fs::write(transaction.join("new/app.exe"),b"new").unwrap();
        fs::write(transaction.join("new/resources/worker.exe"),b"worker").unwrap();
        (temp,app,transaction)
    }
    #[test]
    fn interrupted_apply_and_failed_health_restore_original_files() {
        for partial in [true,false] {
            let (_temp,app,directory)=fixture();
            let mut transaction=Transaction::prepare(&app,&directory,"2.0.0").unwrap();
            assert_eq!(fs::read(app.join("app.exe")).unwrap(),b"old");
            if partial { fs::write(app.join("app.exe"),b"new").unwrap(); }
            else { transaction.apply().unwrap(); assert!(app.join("resources/worker.exe").exists()); }
            drop(transaction);
            Transaction::recover(&app,&directory).unwrap();
            assert_eq!(fs::read(app.join("app.exe")).unwrap(),b"old");
            assert_eq!(fs::read(app.join("user.config")).unwrap(),b"preserve");
            assert!(!app.join("resources/worker.exe").exists());
            Transaction::recover(&app,&directory).unwrap();
        }
    }
    #[test]
    fn confirmed_update_survives_recovery_and_cannot_confirm_without_apply() {
        let (_temp,app,directory)=fixture();
        let mut transaction=Transaction::prepare(&app,&directory,"2.0.0").unwrap();
        assert!(transaction.confirm().is_err());
        transaction.apply().unwrap();transaction.confirm().unwrap();
        Transaction::recover(&app,&directory).unwrap();
        assert_eq!(fs::read(app.join("app.exe")).unwrap(),b"new");
        assert_eq!(fs::read(app.join("resources/worker.exe")).unwrap(),b"worker");
    }
    #[test]
    fn corrupt_backup_is_rejected_before_restore_changes_any_file() {
        let (_temp,app,directory)=fixture();
        let mut transaction=Transaction::prepare(&app,&directory,"2.0.0").unwrap();
        transaction.apply().unwrap();
        fs::write(directory.join("old/app.exe"),b"corrupt").unwrap();
        assert!(Transaction::recover(&app,&directory).is_err());
        assert_eq!(fs::read(app.join("app.exe")).unwrap(),b"new");
        assert!(app.join("resources/worker.exe").exists());
    }
}
