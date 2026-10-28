//! Signed release metadata and payload verification, before any executable switch.
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};

use ferry_protocol::release::{Artifact, MAX_CATALOG, MAX_PAYLOAD, verify_catalog};

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
    #[cfg(unix)]
    std::fs::File::open(root)?.sync_all()?;
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
    // Persist directory entries before publishing a version that the journal
    // may make current. File data fsync alone does not persist their names.
    // Unix chmod above also needs persistence. Windows already flushed the
    // writable output handle; FlushFileBuffers rejects a read-only handle.
    #[cfg(unix)]
    std::fs::File::open(&binary)?.sync_all()?;
    #[cfg(unix)]
    std::fs::File::open(directory.path())?.sync_all()?;
    let target = base.join(&installed.digest);
    if target.exists() {
        staged_binary(root, &installed)?;
    } else {
        durable_rename(directory.path(), &target, false)?;
        #[cfg(unix)]
        std::fs::File::open(&base)?.sync_all()?;
    }
    Ok(installed)
}

#[derive(Debug)]
pub(crate) struct Prepared(pub(crate) Installed);
impl std::fmt::Display for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "更新已验证并暂存")
    }
}
impl std::error::Error for Prepared {}

pub(crate) async fn download(
    root: &Path,
    manifest: &str,
    signature: &str,
    version: &str,
) -> anyhow::Result<Installed> {
    use std::io::Write;
    let artifact = verify_catalog(
        manifest.as_bytes(),
        signature,
        env!("RUNNER_UPDATE_PUBLIC_KEY"),
        env!("RUNNER_BUILD_TARGET"),
        env!("CARGO_PKG_VERSION"),
        version,
    )?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()?;
    let mut response = client.get(&artifact.url).send().await?.error_for_status()?;
    if let Some(size) = response.content_length() {
        ensure!(size == artifact.size, "更新下载长度与签名不一致");
    }
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    let mut received = 0u64;
    while let Some(chunk) = response.chunk().await? {
        received += chunk.len() as u64;
        ensure!(received <= artifact.size, "更新文件超过签名大小");
        file.write_all(&chunk)?;
    }
    file.as_file().sync_all()?;
    stage(root, manifest.as_bytes(), signature, file.path(), version)
}

pub(crate) fn durable_rename(source: &Path, target: &Path, replace: bool) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        let flags = MOVEFILE_WRITE_THROUGH
            | if replace {
                MOVEFILE_REPLACE_EXISTING
            } else {
                0
            };
        ensure!(
            unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), flags) } != 0,
            "版本状态持久化失败：{}",
            std::io::Error::last_os_error()
        );
    }
    #[cfg(not(windows))]
    {
        let _ = replace;
        std::fs::rename(source, target)?;
    }
    Ok(())
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
