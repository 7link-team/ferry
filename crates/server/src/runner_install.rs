//! Agent bootstrap metadata reuses the signed Runner release catalog.
fn metadata(
    bytes: &[u8],
    signature: &str,
    key: &str,
    target: &str,
    version: &str,
) -> anyhow::Result<String> {
    let _ = (bytes, signature, key, target, version);
    anyhow::bail!("runner bootstrap metadata not implemented")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires scripts/test-runner-update.py signing fixtures"]
    fn bootstrap_uses_verified_exact_release_and_never_returns_untrusted_metadata() {
        let root = std::path::PathBuf::from(std::env::var_os("RUNNER_UPDATE_TEST_DIR").unwrap());
        let bytes = std::fs::read(root.join("valid.json")).unwrap();
        let sig = std::fs::read_to_string(root.join("valid.json.sig")).unwrap();
        let key = std::fs::read_to_string(root.join("test.key.pub")).unwrap();
        let output = metadata(&bytes, &sig, &key, "fixture-target", "9.9.9").unwrap();
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "9.9.9");
        assert_eq!(lines[1], "https://example.invalid/runner");
        assert_eq!(lines[3], "31");
        assert!(metadata(b"tampered", &sig, &key, "fixture-target", "9.9.9").is_err());
        assert!(metadata(&bytes, &sig, &key, "fixture-target", "9.9.10").is_err());
        assert!(metadata(&bytes, &sig, &key, "unsupported", "9.9.9").is_err());
    }
}
