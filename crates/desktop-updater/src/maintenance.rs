//! Windows installation-wide exclusion shared by desktop and updater.
use std::{
    fs::File,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};

/// The installer provisions this file outside the replaceable application files.
/// Desktop users receive read permission; only the updater may open it for write.
pub struct InstallationLock {
    _handle: File,
}

impl InstallationLock {
    pub fn running(path: &Path) -> anyhow::Result<Self> {
        Self::open(path, false)
    }
    pub fn updating(path: &Path) -> anyhow::Result<Self> {
        Self::open(path, true)
    }
    fn open(path: &Path, updating: bool) -> anyhow::Result<Self> {
        // Sharing is enforced atomically by the kernel across user sessions.
        // Do not create/truncate: an absent registration must fail closed.
        let handle = File::options()
            .read(true)
            .write(updating)
            .share_mode(if updating { 0 } else { FILE_SHARE_READ })
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let metadata = handle.metadata()?;
        anyhow::ensure!(
            metadata.is_file() && metadata.file_attributes() & 0x400 == 0,
            "安装锁不能是目录或重解析点"
        );
        Ok(Self { _handle: handle })
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
