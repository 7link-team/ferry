//! Windows installation-wide exclusion shared by desktop and updater.
use std::{fs::File, path::Path};

/// The installer provisions this file outside the replaceable application files.
/// Desktop users receive read permission; only the updater may open it for write.
pub struct InstallationLock(File);

impl InstallationLock {
    pub fn running(_path: &Path) -> anyhow::Result<Self> {
        anyhow::bail!("installation lock not implemented")
    }
    pub fn updating(_path: &Path) -> anyhow::Result<Self> {
        anyhow::bail!("installation lock not implemented")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_sessions_block_update_and_update_blocks_new_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("installation.lock");
        std::fs::write(&path, b"registered installation").unwrap();
        let first = InstallationLock::running(&path).unwrap();
        let second = InstallationLock::running(&path).unwrap();
        assert!(InstallationLock::updating(&path).is_err());
        assert!(std::fs::remove_file(&path).is_err());
        drop(first);
        assert!(InstallationLock::updating(&path).is_err());
        drop(second);
        let maintenance = InstallationLock::updating(&path).unwrap();
        assert!(InstallationLock::running(&path).is_err());
        assert!(InstallationLock::updating(&path).is_err());
        assert!(std::fs::remove_file(&path).is_err());
        drop(maintenance);
        let _running = InstallationLock::running(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"registered installation");
    }

    #[test]
    fn missing_or_directory_lock_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        assert!(InstallationLock::running(&missing).is_err());
        assert!(InstallationLock::updating(&missing).is_err());
        assert!(!missing.exists());
        assert!(InstallationLock::running(temp.path()).is_err());
        assert!(InstallationLock::updating(temp.path()).is_err());
    }
}
