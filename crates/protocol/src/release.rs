//! Signed Runner catalog contract, shared by bootstrap and upgrades.
use anyhow::{Context, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read};

pub const MAX_CATALOG: usize = 64 * 1024;
pub const MAX_PAYLOAD: u64 = 256 * 1024 * 1024;

/// Stable identity for application validation failures; cryptographic checks are unchanged.
#[derive(Debug)]
pub struct ValidationError {
    pub code: &'static str,
    fallback: &'static str,
}

impl ValidationError {
    pub const fn new(code: &'static str, fallback: &'static str) -> Self {
        Self { code, fallback }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.fallback)
    }
}
impl std::error::Error for ValidationError {}

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
    ensure!(
        value.len() <= 8192,
        ValidationError::new(
            "signatureSize",
            "The update signature or public key is too large."
        )
    );
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
        ValidationError::new("manifestSize", "The update manifest size is invalid.")
    );
    let key = minisign_verify::PublicKey::decode(&decoded(public_key)?)?;
    let signature = minisign_verify::Signature::decode(&decoded(signature)?)?;
    key.verify(bytes, &signature, false)
        .context(ValidationError::new(
            "signatureInvalid",
            "The update manifest signature is invalid.",
        ))?;
    // Metadata is trusted only after verifying the exact original bytes.
    let mut catalog: Catalog = serde_json::from_slice(bytes)?;
    ensure!(
        catalog.schema == 1,
        ValidationError::new("schema", "The update manifest version is unsupported.")
    );
    ensure!(
        catalog.protocol == crate::VERSION,
        ValidationError::new("protocol", "The update protocol is incompatible.")
    );
    ensure!(
        catalog.version == expected,
        ValidationError::new("version", "The update version does not match the request.")
    );
    ensure!(
        semver::Version::parse(&catalog.version)?
            .cmp_precedence(&semver::Version::parse(current)?)
            .is_gt(),
        ValidationError::new(
            "upgradeOnly",
            "The update version must be newer than the current version."
        )
    );
    let artifact = catalog
        .targets
        .remove(target)
        .context(ValidationError::new(
            "target",
            "The update does not support this platform architecture.",
        ))?;
    ensure!(
        artifact.size > 0 && artifact.size <= MAX_PAYLOAD,
        ValidationError::new("payloadSize", "The update file size is invalid.")
    );
    ensure!(
        artifact.sha256.len() == 64
            && artifact
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        ValidationError::new("hash", "The update file hash is invalid.")
    );
    let url = url::Url::parse(&artifact.url)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        ValidationError::new("url", "The update URL must use HTTPS without credentials.")
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
            ensure!(
                length <= self.size,
                ValidationError::new(
                    "payloadOversize",
                    "The update file exceeds its signed manifest size."
                )
            );
            hash.update(&buffer[..count]);
        }
        ensure!(
            length == self.size,
            ValidationError::new(
                "payloadIncomplete",
                "The update file was not fully downloaded."
            )
        );
        ensure!(
            format!("{:x}", hash.finalize()) == self.sha256,
            ValidationError::new("payloadHash", "The update file hash does not match.")
        );
        Ok(())
    }
}
