//! Long-lived host identity persisted across enrollment and reconnects.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What survives a restart. Written with 0600: it is the host's credential.
#[derive(Serialize, Deserialize)]
pub(super) struct Enrolment {
    pub(super) host_id: String,
    pub(super) key: String,
}

impl Enrolment {
    pub(super) fn path(state_dir: &std::path::Path) -> PathBuf {
        state_dir.join("runner.json")
    }

    pub(super) fn load(state_dir: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(Self::path(state_dir)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub(super) fn save(&self, state_dir: &std::path::Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(state_dir)
            .with_context(|| format!("无法创建状态目录 {}", state_dir.display()))?;
        let path = Self::path(state_dir);
        std::fs::write(&path, serde_json::to_vec(self)?)
            .with_context(|| format!("无法写入 {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_enrolment_file_survives_a_round_trip() {
        let dir = std::env::temp_dir().join(format!("ferry-runner-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(Enrolment::load(&dir).is_none(), "nothing stored yet");
        Enrolment {
            host_id: "h-1".into(),
            key: "k-1".into(),
        }
        .save(&dir)
        .unwrap();
        let back = Enrolment::load(&dir).expect("stored");
        assert_eq!((back.host_id.as_str(), back.key.as_str()), ("h-1", "k-1"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(Enrolment::path(&dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "the key must not be world-readable");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
