//! Bounded package transfer for the local helper/broker connection.
use std::path::Path;
use tokio::io::AsyncRead;

pub struct VerifiedDownload {
    version: String,
    file: tempfile::NamedTempFile,
}
impl VerifiedDownload {
    pub fn version(&self) -> &str {
        &self.version
    }
    pub fn file(&self) -> &std::fs::File {
        self.file.as_file()
    }
}

/// The caller owns and protects the staging directory and supplies the
/// installed version from trusted registration. Neither comes from the peer.
pub async fn receive(
    _source: impl AsyncRead + Unpin,
    _directory: &Path,
    _current: &str,
) -> anyhow::Result<VerifiedDownload> {
    anyhow::bail!("package transfer not implemented")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn frame(header: serde_json::Value, bytes: &[u8]) -> Vec<u8> {
        let header = serde_json::to_vec(&header).unwrap();
        let mut framed = (header.len() as u32).to_le_bytes().to_vec();
        framed.extend(header);
        framed.extend(bytes);
        framed
    }
    #[tokio::test]
    #[ignore = "requires CI signing fixture"]
    async fn signed_transfer_is_bounded_and_failed_transfers_leave_no_staged_files() {
        let fixtures =
            std::path::PathBuf::from(std::env::var_os("DESKTOP_UPDATE_TEST_DIR").unwrap());
        let name = format!(
            "{}_9.9.9_windows-x86_64-update.zip",
            env!("DESKTOP_PRODUCT")
        );
        let bytes = std::fs::read(fixtures.join(&name)).unwrap();
        let signature = std::fs::read_to_string(fixtures.join(format!("{name}.sig"))).unwrap();
        let header = json!({"version":"9.9.9","signature":signature,"size":bytes.len()});
        let directory = tempfile::tempdir().unwrap();
        let data = frame(header.clone(), &bytes);
        let received = receive(data.as_slice(), directory.path(), "1.0.0")
            .await
            .unwrap();
        assert_eq!(received.version(), "9.9.9");
        use std::io::{Read, Seek};
        let mut actual = Vec::new();
        let mut file = received.file();
        file.rewind().unwrap();
        file.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, bytes);
        drop(received);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        let mut commands = header.clone();
        commands["command"] = json!("execute");
        let mut destination = header.clone();
        destination["destination"] = json!("C:/Windows");
        let mut huge = header.clone();
        huge["size"] = json!(2u64 * 1024 * 1024 * 1024 + 1);
        let mut empty = header.clone();
        empty["size"] = json!(0);
        let mut wrong_version = header.clone();
        wrong_version["version"] = json!("9.9.10");
        let mut too_long = header.clone();
        too_long["signature"] = json!("x".repeat(8193));
        for malformed in [
            frame(commands, &bytes),
            frame(destination, &bytes),
            frame(huge, &bytes),
            frame(empty, &bytes),
            frame(wrong_version, &bytes),
            frame(too_long, &bytes),
            frame(header.clone(), &bytes[..bytes.len() - 1]),
            frame(header.clone(), &vec![b'x'; bytes.len()]),
            u32::MAX.to_le_bytes().to_vec(),
            vec![0, 0],
        ] {
            assert!(
                receive(malformed.as_slice(), directory.path(), "1.0.0")
                    .await
                    .is_err()
            );
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        }
        assert!(
            receive(data.as_slice(), directory.path(), "9.9.9")
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
