//! Encryption for everything the product must store but must never show:
//! SSH passwords and private keys, Git tokens, and the secret half of a
//! workspace's variables.
//!
//! One master key, XChaCha20-Poly1305, a fresh random nonce per secret, and
//! the row's identity as associated data. The AAD is the part that is easy to
//! leave out and expensive to miss: without it a ciphertext copied from one
//! credential row into another — or into another workspace's row — would still
//! decrypt. With it, a moved ciphertext fails to open.
//!
//! The master key comes from `FERRY_MASTER_KEY` (Docker) or a 0600 file in the
//! data directory (desktop). A deployment that configures neither is refused
//! at startup rather than quietly storing secrets in the clear.
//
// ponytail: one master key, no per-workspace key-wrapping layer. The design
// doc's wrapped-DEK scheme buys crypto-shredding a single workspace by
// destroying its key; nothing needs that yet. Add the layer when workspace
// deletion has to be provably irreversible.

use crate::error::{ApiError, Result};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand::RngCore;
use std::path::Path;

const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
/// Bumped if the scheme ever changes, so old rows stay readable.
const VERSION: u8 = 1;

#[derive(Clone)]
pub struct Vault {
    key: [u8; KEY_BYTES],
}

impl Vault {
    /// `FERRY_MASTER_KEY` wins; otherwise a key file beside the database, which
    /// is generated on first run for the desktop build. `server_mode` is the
    /// Docker deployment, where generating a key on a throwaway container
    /// filesystem would silently make every stored secret unreadable after a
    /// restart — so there it must be supplied.
    pub fn open(data_dir: &Path, server_mode: bool) -> anyhow::Result<Self> {
        if let Ok(configured) = std::env::var("FERRY_MASTER_KEY") {
            let raw = configured.trim();
            anyhow::ensure!(
                !raw.is_empty(),
                "FERRY_MASTER_KEY 为空。请用 `openssl rand -hex 32` 生成一个 64 位十六进制密钥。"
            );
            let bytes = hex::decode(raw).map_err(|_| {
                anyhow::anyhow!("FERRY_MASTER_KEY 必须是 64 位十六进制字符（32 字节）。")
            })?;
            let key: [u8; KEY_BYTES] = bytes.as_slice().try_into().map_err(|_| {
                anyhow::anyhow!(
                    "FERRY_MASTER_KEY 必须是 32 字节，当前为 {} 字节。",
                    bytes.len()
                )
            })?;
            return Ok(Self { key });
        }

        let path = data_dir.join("master.key");
        if let Ok(stored) = std::fs::read_to_string(&path) {
            let bytes = hex::decode(stored.trim())
                .map_err(|_| anyhow::anyhow!("{} 内容损坏，无法解析为密钥。", path.display()))?;
            let key: [u8; KEY_BYTES] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("{} 不是 32 字节密钥。", path.display()))?;
            return Ok(Self { key });
        }

        anyhow::ensure!(
            !server_mode,
            "未配置 FERRY_MASTER_KEY。容器重启会丢失自动生成的密钥，届时所有已保存的凭据都无法解密，\
             因此必须显式提供：用 `openssl rand -hex 32` 生成，再通过环境变量传入。"
        );
        let mut key = [0u8; KEY_BYTES];
        rand::thread_rng().fill_bytes(&mut key);
        write_private(&path, &hex::encode(key))?;
        tracing::info!(path = %path.display(), "已生成本机主密钥");
        Ok(Self { key })
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&self.key).into())
    }

    /// `v1.<nonce-hex>.<ciphertext-hex>`. `aad` binds the ciphertext to the row
    /// it belongs to; pass the same value to `open_secret`.
    pub fn seal(&self, plaintext: &str, aad: &str) -> Result<String> {
        let mut nonce = [0u8; NONCE_BYTES];
        rand::thread_rng().fill_bytes(&mut nonce);
        let sealed = self
            .cipher()
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| {
                ApiError(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "加密失败，凭据未保存。".into(),
                )
            })?;
        Ok(format!(
            "v{VERSION}.{}.{}",
            hex::encode(nonce),
            hex::encode(sealed)
        ))
    }

    pub fn open_secret(&self, stored: &str, aad: &str) -> Result<String> {
        let broken = || {
            ApiError(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "凭据无法解密。".into(),
            )
        };
        let mut parts = stored.splitn(3, '.');
        let version = parts.next().unwrap_or_default();
        let (Some(nonce), Some(body)) = (parts.next(), parts.next()) else {
            return Err(broken());
        };
        if version != format!("v{VERSION}") {
            return Err(broken());
        }
        let nonce = hex::decode(nonce).map_err(|_| broken())?;
        let body = hex::decode(body).map_err(|_| broken())?;
        if nonce.len() != NONCE_BYTES {
            return Err(broken());
        }
        let plain = self
            .cipher()
            .decrypt(
                &XNonce::from(<[u8; NONCE_BYTES]>::try_from(&nonce[..]).map_err(|_| broken())?),
                Payload {
                    msg: &body,
                    aad: aad.as_bytes(),
                },
            )
            // A wrong master key and a tampered or relocated row are the same
            // failure here on purpose: the response must not say which.
            .map_err(|_| broken())?;
        String::from_utf8(plain).map_err(|_| broken())
    }
}

/// The identity a secret is bound to. Changing the shape invalidates existing
/// rows, so it is written once, here.
pub fn aad(workspace_id: &str, resource: &str, id: &str) -> String {
    format!("{workspace_id}/{resource}/{id}")
}

fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> Vault {
        Vault {
            key: [7u8; KEY_BYTES],
        }
    }

    #[test]
    fn a_secret_round_trips() {
        let v = vault();
        let sealed = v
            .seal("hunter2-correct-horse", "ws/credentials/c1")
            .unwrap();
        assert_eq!(
            v.open_secret(&sealed, "ws/credentials/c1").unwrap(),
            "hunter2-correct-horse"
        );
    }

    #[test]
    fn the_stored_form_never_contains_the_plaintext() {
        let sealed = vault()
            .seal("super-secret-value", "ws/credentials/c1")
            .unwrap();
        assert!(!sealed.contains("super-secret-value"));
        assert!(sealed.starts_with("v1."));
    }

    #[test]
    fn the_same_plaintext_seals_differently_every_time() {
        // A deterministic ciphertext would leak that two hosts share a password.
        let v = vault();
        let a = v.seal("same", "ws/credentials/c1").unwrap();
        let b = v.seal("same", "ws/credentials/c1").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn a_ciphertext_moved_to_another_row_will_not_open() {
        let v = vault();
        let sealed = v.seal("secret", &aad("ws-a", "credentials", "c1")).unwrap();
        // Same vault, different row: this is the attack the AAD exists to stop.
        assert!(
            v.open_secret(&sealed, &aad("ws-a", "credentials", "c2"))
                .is_err()
        );
        assert!(
            v.open_secret(&sealed, &aad("ws-b", "credentials", "c1"))
                .is_err()
        );
        assert!(
            v.open_secret(&sealed, &aad("ws-a", "variables", "c1"))
                .is_err()
        );
        // And the row it belongs to still opens.
        assert_eq!(
            v.open_secret(&sealed, &aad("ws-a", "credentials", "c1"))
                .unwrap(),
            "secret"
        );
    }

    #[test]
    fn another_master_key_cannot_read_it() {
        let sealed = vault().seal("secret", "aad").unwrap();
        let other = Vault {
            key: [9u8; KEY_BYTES],
        };
        assert!(other.open_secret(&sealed, "aad").is_err());
    }

    #[test]
    fn tampering_is_detected_rather_than_returning_garbage() {
        let v = vault();
        let sealed = v.seal("secret", "aad").unwrap();
        // Flip the last ciphertext nibble.
        let mut bytes: Vec<char> = sealed.chars().collect();
        let last = bytes.len() - 1;
        bytes[last] = if bytes[last] == '0' { '1' } else { '0' };
        let tampered: String = bytes.into_iter().collect();
        assert!(v.open_secret(&tampered, "aad").is_err());
    }

    #[test]
    fn malformed_stored_values_are_refused_without_panicking() {
        let v = vault();
        for bad in [
            "",
            "v1",
            "v1.",
            "v1.zz.zz",
            "v2.00.00",
            "garbage",
            "v1.00.",
            &"v1.".repeat(9),
        ] {
            assert!(v.open_secret(bad, "aad").is_err(), "{bad}");
        }
    }
}
