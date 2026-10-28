//! Admission policy shared by the unelevated helper and privileged broker.
use std::io::Read;
#[cfg(windows)]
pub mod maintenance;
pub mod transaction;

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

/// Expand a verified archive into a fresh, caller-owned protected directory.
/// Validate all names before creating any files; never follow archive links.
pub fn extract_package(
    source: impl Read + std::io::Seek,
    destination: &std::path::Path,
) -> anyhow::Result<()> {
    use anyhow::ensure;
    use std::{collections::BTreeSet, fs};
    let metadata = fs::symlink_metadata(destination)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "更新暂存目录无效"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "更新目录不能是重解析点"
        );
    }
    ensure!(
        fs::read_dir(destination)?.next().is_none(),
        "更新暂存目录必须为空"
    );
    let mut archive = zip::ZipArchive::new(source)?;
    ensure!(!archive.has_duplicate_names(), "更新包存在重复文件名");
    ensure!(
        !archive.is_empty() && archive.len() <= 16384,
        "更新包文件数量无效"
    );
    let mut names = BTreeSet::new();
    let mut entries = Vec::new();
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let entry_name = entry.name()?;
        let name = entry_name.strip_suffix('/').unwrap_or(&entry_name);
        let path = safe_entry_path(name)?;
        ensure!(names.insert(name.to_lowercase()), "更新包存在重复文件名");
        let kind = entry.unix_mode().unwrap_or(0) & 0o170000;
        ensure!(
            kind == 0
                || (kind == 0o100000 && !entry.is_dir())
                || (kind == 0o040000 && entry.is_dir()),
            "更新包不能包含链接或特殊文件"
        );
        expanded = expanded
            .checked_add(entry.size())
            .ok_or_else(|| anyhow::anyhow!("更新包解压体积溢出"))?;
        ensure!(expanded <= MAX_PACKAGE_BYTES, "更新包解压体积过大");
        entries.push((path, entry.is_dir(), entry.size()));
    }
    let mut directories = BTreeSet::new();
    for (index, (path, directory, size)) in entries.into_iter().enumerate() {
        let target = destination.join(path);
        let parent = if directory {
            target.as_path()
        } else {
            target.parent().unwrap()
        };
        fs::create_dir_all(parent)?;
        for ancestor in parent
            .ancestors()
            .take_while(|path| path.starts_with(destination))
        {
            directories.insert(ancestor.to_path_buf());
        }
        if directory {
            continue;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)?;
        let mut entry = archive.by_index(index)?.take(size + 1);
        let copied = std::io::copy(&mut entry, &mut file)?;
        ensure!(copied == size, "更新包文件长度不符");
        file.sync_all()?;
    }
    #[cfg(unix)]
    for directory in directories.iter().rev() {
        fs::File::open(directory)?.sync_all()?;
    }
    Ok(())
}

fn safe_entry_path(name: &str) -> anyhow::Result<std::path::PathBuf> {
    use anyhow::ensure;
    ensure!(
        !name.is_empty() && name.encode_utf16().count() <= 240,
        "更新包文件路径过长或为空"
    );
    ensure!(
        !name
            .chars()
            .any(|c| c.is_control() || r#"\:<>"|?*~"#.contains(c)),
        "更新包包含非法 Windows 路径"
    );
    let mut path = std::path::PathBuf::new();
    for component in name.split('/') {
        ensure!(
            !component.is_empty() && !component.ends_with(['.', ' ']),
            "更新包包含路径穿越或别名"
        );
        let upper = component.split('.').next().unwrap().to_uppercase();
        let device = matches!(
            upper.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            upper.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
        ensure!(
            !device && !component.eq_ignore_ascii_case(".desktop-update-lock"),
            "更新包使用保留名称"
        );
        path.push(component);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[ignore = "scripts/test-desktop-update-policy.py supplies archive fixtures"]
    fn archive_staging_rejects_windows_path_aliases_and_special_files() {
        let root = PathBuf::from(std::env::var("DESKTOP_UPDATE_TEST_DIR").unwrap());
        let stage = tempfile::tempdir().unwrap();
        extract_package(
            std::fs::File::open(root.join("valid.zip")).unwrap(),
            stage.path(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read(stage.path().join("application.exe")).unwrap(),
            b"application"
        );
        assert_eq!(
            std::fs::read(stage.path().join("resources/worker.exe")).unwrap(),
            b"worker"
        );
        assert!(
            extract_package(
                std::fs::File::open(root.join("valid.zip")).unwrap(),
                stage.path()
            )
            .is_err(),
            "existing staged files must not be overwritten"
        );
        for name in [
            "traversal",
            "absolute",
            "drive",
            "backslash",
            "reserved",
            "trailing",
            "alternate-stream",
            "case-alias",
            "symlink",
            "oversize",
            "duplicate",
        ] {
            let stage = tempfile::tempdir().unwrap();
            assert!(
                extract_package(
                    std::fs::File::open(root.join(format!("{name}.zip"))).unwrap(),
                    stage.path()
                )
                .is_err(),
                "must reject {name}"
            );
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
