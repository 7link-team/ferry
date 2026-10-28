//! Admission policy shared by the unelevated helper and privileged broker.
use std::io::Read;

pub const MAX_PACKAGE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Verify original bytes before trusting the signed product/version fields.
/// Callers must use an immutable protected file for subsequent extraction.
pub fn verify_package(
    mut source: impl Read,
    signature: &str,
    version: &str,
    current: &str,
) -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use base64::{Engine, engine::general_purpose::STANDARD};
    ensure!(
        signature.len() <= 8192 && version.len() <= 128 && current.len() <= 128,
        "更新元数据过大"
    );
    let expected = semver::Version::parse(version)?;
    ensure!(
        expected
            .cmp_precedence(&semver::Version::parse(current)?)
            .is_gt(),
        "更新版本必须高于当前版本"
    );
    let key_text = String::from_utf8(STANDARD.decode(env!("DESKTOP_UPDATE_PUBLIC_KEY").trim())?)?;
    let key = minisign_verify::PublicKey::decode(&key_text)?;
    let signature_text = String::from_utf8(STANDARD.decode(signature.trim())?)?;
    let signature = minisign_verify::Signature::decode(&signature_text)?;
    let mut verifier = key.verify_stream(&signature)?;
    let mut buffer = [0u8; 65536];
    let mut size = 0u64;
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        ensure!(size <= MAX_PACKAGE_BYTES, "更新包超过大小限制");
        verifier.update(&buffer[..count]);
    }
    ensure!(size > 0, "更新包为空");
    verifier.finalize().context("更新包签名无效")?;
    let comment = signature.trusted_comment();
    let field = |prefix: &str| -> anyhow::Result<&str> {
        let mut values = comment
            .split('\t')
            .filter_map(|part| part.strip_prefix(prefix));
        let value = values.next().context("签名缺少产品或版本绑定")?;
        ensure!(values.next().is_none(), "签名包含重复身份字段");
        Ok(value)
    };
    ensure!(field("version:")? == version, "签名版本与请求不一致");
    let filename = format!(
        "{}_{}_windows-x86_64-update.zip",
        env!("DESKTOP_PRODUCT"),
        version
    );
    ensure!(
        field("file:")? == filename,
        "更新包不是本产品的 Windows 更新文件"
    );
    Ok(())
}

/// Expand a verified archive into a fresh protected staging directory.
pub fn extract_package(_source: impl Read + std::io::Seek, _destination: &std::path::Path) -> anyhow::Result<()> {
    anyhow::bail!("desktop package extraction is not implemented")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[ignore = "scripts/test-desktop-update-policy.py supplies archive fixtures"]
    fn archive_staging_rejects_windows_path_aliases_and_special_files() {
        let root=PathBuf::from(std::env::var("DESKTOP_UPDATE_TEST_DIR").unwrap());
        let stage=tempfile::tempdir().unwrap();
        extract_package(std::fs::File::open(root.join("valid.zip")).unwrap(),stage.path()).unwrap();
        assert_eq!(std::fs::read(stage.path().join("application.exe")).unwrap(),b"application");
        assert_eq!(std::fs::read(stage.path().join("resources/worker.exe")).unwrap(),b"worker");
        assert!(extract_package(std::fs::File::open(root.join("valid.zip")).unwrap(),stage.path()).is_err(),"existing staged files must not be overwritten");
        for name in ["traversal","absolute","drive","backslash","reserved","trailing","alternate-stream","case-alias","symlink","oversize","duplicate"] {
            let stage=tempfile::tempdir().unwrap();
            assert!(extract_package(std::fs::File::open(root.join(format!("{name}.zip"))).unwrap(),stage.path()).is_err(),"must reject {name}");
        }
    }

    #[test]
    #[ignore = "scripts/test-desktop-update-policy.py supplies genuine throwaway signatures"]
    fn signed_windows_package_admission() {
        let root = PathBuf::from(std::env::var("DESKTOP_UPDATE_TEST_DIR").unwrap());
        let check = |name: &str, version: &str, current: &str| {
            verify_package(
                std::fs::File::open(root.join(name)).unwrap(),
                &std::fs::read_to_string(root.join(format!("{name}.sig"))).unwrap(),
                version,
                current,
            )
        };
        let installer = format!(
            "{}_9.9.9_windows-x86_64-update.zip",
            env!("DESKTOP_PRODUCT")
        );
        assert!(
            check(&installer, "9.9.9", "0.1.0-alpha.11").is_ok(),
            "genuine newer update package must be accepted"
        );
        assert!(
            check(&installer, "9.9.10", "0.1.0-alpha.11").is_err(),
            "relabelled version"
        );
        assert!(
            check(&installer, "9.9.9", "9.9.9").is_err(),
            "same version replay"
        );
        assert!(check(&installer, "9.9.9", "10.0.0").is_err(), "downgrade");
        assert!(
            check(
                "other-product_9.9.9_windows-x86_64-update.zip",
                "9.9.9",
                "0.1.0"
            )
            .is_err(),
            "another product signed with same key"
        );
        assert!(
            check("unversioned.exe", "9.9.9", "0.1.0").is_err(),
            "unsigned version"
        );
        assert!(
            check("tampered.exe", "9.9.9", "0.1.0").is_err(),
            "modified payload"
        );
        assert!(
            check("wrong-key.exe", "9.9.9", "0.1.0").is_err(),
            "untrusted key"
        );
        let signature = std::fs::read_to_string(root.join(format!("{installer}.sig"))).unwrap();
        assert!(
            verify_package(std::io::empty(), &signature, "9.9.9", "0.1.0").is_err(),
            "empty/truncated bytes"
        );
        assert!(
            verify_package(std::io::empty(), &"x".repeat(8193), "9.9.9", "0.1.0").is_err(),
            "oversize signature"
        );
        assert!(
            verify_package(std::io::empty(), &signature, &"9".repeat(129), "0.1.0").is_err(),
            "oversize version"
        );
    }
}
