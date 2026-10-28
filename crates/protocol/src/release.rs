//! Signed Runner catalog contract, shared by bootstrap and upgrades.
use anyhow::{Context, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read};

pub const MAX_CATALOG: usize = 64 * 1024;
pub const MAX_PAYLOAD: u64 = 256 * 1024 * 1024;

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
pub struct Artifact {
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

fn decoded(value: &str) -> anyhow::Result<String> {
    ensure!(value.len() <= 8192, "更新签名或公钥过大");
    Ok(String::from_utf8(STANDARD.decode(value.trim())?)?)
}

pub fn verify_catalog(
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
    ensure!(catalog.protocol == crate::VERSION, "更新协议不兼容");
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
    let url = url::Url::parse(&artifact.url)?;
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
    pub fn verify(&self, mut reader: impl Read) -> anyhow::Result<()> {
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
