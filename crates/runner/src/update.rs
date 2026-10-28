//! Signed release metadata and payload verification, before any executable switch.
use anyhow::{Context, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, path::Path};

const MAX_CATALOG: usize = 64 * 1024;
const MAX_PAYLOAD: u64 = 256 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema: u32,
    protocol: u32,
    version: String,
    targets: BTreeMap<String, Artifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Artifact {
    pub(crate) url: String,
    size: u64,
    sha256: String,
}

fn decoded(value: &str) -> anyhow::Result<String> {
    ensure!(value.len() <= 8192, "更新签名或公钥过大");
    Ok(String::from_utf8(STANDARD.decode(value.trim())?)?)
}

fn verify_catalog(
    bytes: &[u8],
    signature: &str,
    public_key: &str,
    target: &str,
    current: &str,
    expected: &str,
) -> anyhow::Result<Artifact> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_CATALOG,
        "更新清单大小无效"
    );
    let key = minisign_verify::PublicKey::decode(&decoded(public_key)?)?;
    let signature = minisign_verify::Signature::decode(&decoded(signature)?)?;
    key.verify(bytes, &signature, false)
        .context("更新清单签名无效")?;
    // Metadata is trusted only after verifying the exact original bytes.
    let mut catalog: Catalog = serde_json::from_slice(bytes)?;
    ensure!(catalog.schema == 1, "不支持的更新清单版本");
    ensure!(
        catalog.protocol == ferry_protocol::VERSION,
        "更新协议不兼容"
    );
    ensure!(catalog.version == expected, "更新版本与请求不一致");
    ensure!(
        semver::Version::parse(&catalog.version)?
            .cmp_precedence(&semver::Version::parse(current)?)
            .is_gt(),
        "更新版本必须高于当前版本"
    );
    let artifact = catalog
        .targets
        .remove(target)
        .context("更新不支持当前平台架构")?;
    ensure!(
        artifact.size > 0 && artifact.size <= MAX_PAYLOAD,
        "更新文件大小无效"
    );
    ensure!(
        artifact.sha256.len() == 64
            && artifact
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "更新文件哈希无效"
    );
    let url = reqwest::Url::parse(&artifact.url)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "更新下载地址必须为无凭据的HTTPS地址"
    );
    Ok(artifact)
}

impl Artifact {
    fn verify(&self, mut reader: impl Read) -> anyhow::Result<()> {
        let mut hash = Sha256::new();
        let mut length = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            length += count as u64;
            ensure!(length <= self.size, "更新文件超出签名清单大小");
            hash.update(&buffer[..count]);
        }
        ensure!(length == self.size, "更新文件未完整下载");
        ensure!(
            format!("{:x}", hash.finalize()) == self.sha256,
            "更新文件哈希不匹配"
        );
        Ok(())
    }
}

pub(crate) fn verify_files(
    manifest: &Path,
    signature: &Path,
    artifact: &Path,
    expected: &str,
) -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    std::fs::File::open(manifest)?
        .take((MAX_CATALOG + 1) as u64)
        .read_to_end(&mut bytes)?;
    let mut sig = String::new();
    std::fs::File::open(signature)?
        .take(8193)
        .read_to_string(&mut sig)?;
    let verified = verify_catalog(
        &bytes,
        &sig,
        env!("RUNNER_UPDATE_PUBLIC_KEY"),
        env!("RUNNER_BUILD_TARGET"),
        env!("CARGO_PKG_VERSION"),
        expected,
    )?;
    verified.verify(std::fs::File::open(artifact)?)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Installed {
    pub(crate) version: String,
    pub(crate) digest: String,
}

fn version_dir(root: &Path, installed: &Installed) -> anyhow::Result<std::path::PathBuf> {
    ensure!(
        installed.digest.len() == 64
            && installed
                .digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "版本目录标识无效"
    );
    let base = root.join("versions");
    let dir = base.join(&installed.digest);
    for path in [&base, &dir] {
        ensure!(
            std::fs::symlink_metadata(path)?.file_type().is_dir(),
            "版本目录不能是符号链接"
        );
    }
    Ok(dir)
}

pub(crate) fn staged_binary(
    root: &Path,
    installed: &Installed,
) -> anyhow::Result<std::path::PathBuf> {
    let dir = version_dir(root, installed)?;
    let bytes = std::fs::read(dir.join("manifest.json"))?;
    let signature = std::fs::read_to_string(dir.join("manifest.sig"))?;
    let artifact = verify_catalog(
        &bytes,
        &signature,
        env!("RUNNER_UPDATE_PUBLIC_KEY"),
        env!("RUNNER_BUILD_TARGET"),
        "0.0.0-0",
        &installed.version,
    )?;
    ensure!(
        artifact.sha256 == installed.digest,
        "版本目录与签名哈希不一致"
    );
    let binary = dir.join(if cfg!(windows) {
        "runner.exe"
    } else {
        "runner"
    });
    ensure!(
        std::fs::symlink_metadata(&binary)?.file_type().is_file(),
        "代理程序必须是普通文件"
    );
    artifact.verify(std::fs::File::open(&binary)?)?;
    Ok(binary)
}

pub(crate) fn stage(
    root: &Path,
    bytes: &[u8],
    signature: &str,
    source: &Path,
    version: &str,
) -> anyhow::Result<Installed> {
    use std::io::Write;
    let artifact = verify_catalog(
        bytes,
        signature,
        env!("RUNNER_UPDATE_PUBLIC_KEY"),
        env!("RUNNER_BUILD_TARGET"),
        env!("CARGO_PKG_VERSION"),
        version,
    )?;
    let installed = Installed {
        version: version.to_owned(),
        digest: artifact.sha256.clone(),
    };
    let base = root.join("versions");
    std::fs::create_dir_all(&base)?;
    ensure!(
        std::fs::symlink_metadata(&base)?.file_type().is_dir(),
        "版本目录不能是符号链接"
    );
    let directory = tempfile::Builder::new()
        .prefix("staging-")
        .tempdir_in(&base)?;
    let binary = directory.path().join(if cfg!(windows) {
        "runner.exe"
    } else {
        "runner"
    });
    let mut input = std::fs::File::open(source)?.take(artifact.size + 1);
    let mut output = std::fs::File::create(&binary)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    drop(output);
    artifact.verify(std::fs::File::open(&binary)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))?;
    }
    for (name, data) in [
        ("manifest.json", bytes),
        ("manifest.sig", signature.as_bytes()),
    ] {
        let mut file = std::fs::File::create(directory.path().join(name))?;
        file.write_all(data)?;
        file.sync_all()?;
    }
    let target = base.join(&installed.digest);
    if target.exists() {
        staged_binary(root, &installed)?;
    } else {
        std::fs::rename(directory.path(), &target)?;
        #[cfg(unix)]
        std::fs::File::open(&base)?.sync_all()?;
    }
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "run scripts/test-runner-update.py with throwaway signed fixtures"]
    fn signed_runner_update_rejects_tampering_and_wrong_metadata() {
        let root = std::path::PathBuf::from(std::env::var("RUNNER_UPDATE_TEST_DIR").unwrap());
        let key = std::fs::read_to_string(root.join("test.key.pub")).unwrap();
        let other = std::fs::read_to_string(root.join("other.key.pub")).unwrap();
        let bytes = std::fs::read(root.join("valid.json")).unwrap();
        let sig = std::fs::read_to_string(root.join("valid.json.sig")).unwrap();
        let check =
            |data: &[u8], sig: &str, key: &str, target: &str, current: &str, expected: &str| {
                verify_catalog(data, sig, key, target, current, expected)
            };
        let valid = check(
            &bytes,
            &sig,
            &key,
            "fixture-target",
            "0.1.0-alpha.11",
            "9.9.9",
        )
        .unwrap();
        let payload = std::fs::read(root.join("payload.bin")).unwrap();
        valid.verify(payload.as_slice()).unwrap();
        let mut damaged = payload.clone();
        damaged[0] ^= 1;
        assert!(valid.verify(damaged.as_slice()).is_err());
        assert!(valid.verify(&payload[..payload.len() - 1]).is_err());
        let mut oversized = payload.clone();
        oversized.push(0);
        assert!(valid.verify(oversized.as_slice()).is_err());
        let mut tampered = bytes.clone();
        tampered[0] ^= 1;
        assert!(check(&tampered, &sig, &key, "fixture-target", "0.1.0", "9.9.9").is_err());
        assert!(check(&bytes, &sig, &other, "fixture-target", "0.1.0", "9.9.9").is_err());
        assert!(check(&bytes, &sig, &key, "wrong-target", "0.1.0", "9.9.9").is_err());
        assert!(check(&bytes, &sig, &key, "fixture-target", "0.1.0", "9.9.10").is_err());
        assert!(check(&bytes, &sig, &key, "fixture-target", "9.9.9", "9.9.9").is_err());
        for invalid in [
            "bad-url",
            "credential-url",
            "bad-size",
            "oversize",
            "bad-hash",
        ] {
            let data = std::fs::read(root.join(format!("{invalid}.json"))).unwrap();
            let signature =
                std::fs::read_to_string(root.join(format!("{invalid}.json.sig"))).unwrap();
            assert!(
                check(&data, &signature, &key, "fixture-target", "0.1.0", "9.9.9").is_err(),
                "{invalid}"
            );
        }
        assert!(
            check(
                &vec![0; MAX_CATALOG + 1],
                &sig,
                &key,
                "fixture-target",
                "0.1.0",
                "9.9.9"
            )
            .is_err()
        );
        assert!(
            check(
                &bytes,
                &"X".repeat(8193),
                &key,
                "fixture-target",
                "0.1.0",
                "9.9.9"
            )
            .is_err()
        );
        assert!(
            check(
                &bytes,
                "invalid-signature",
                &key,
                "fixture-target",
                "0.1.0",
                "9.9.9"
            )
            .is_err()
        );
        for (name, version) in [
            ("wrong-protocol", "9.9.9"),
            ("wrong-schema", "9.9.9"),
            ("downgrade", "0.0.1"),
        ] {
            let data = std::fs::read(root.join(format!("{name}.json"))).unwrap();
            let signature = std::fs::read_to_string(root.join(format!("{name}.json.sig"))).unwrap();
            assert!(
                check(&data, &signature, &key, "fixture-target", "0.1.0", version).is_err(),
                "{name}"
            );
        }
    }
}
