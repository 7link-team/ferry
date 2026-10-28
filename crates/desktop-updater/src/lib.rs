//! Admission policy shared by the unelevated helper and privileged broker.
use std::io::Read;

/// Verify bytes and signed installer identity before any privileged execution.
pub fn verify_installer(_source: impl Read, _signature: &str, _version: &str, _current: &str) -> anyhow::Result<()> {
    anyhow::bail!("desktop broker admission is not implemented")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    #[ignore = "scripts/test-desktop-update-policy.py supplies genuine throwaway signatures"]
    fn signed_windows_installer_admission() {
        let root=PathBuf::from(std::env::var("DESKTOP_UPDATE_TEST_DIR").unwrap());
        let check=|name:&str, version:&str, current:&str| {
            verify_installer(std::fs::File::open(root.join(name)).unwrap(),
                &std::fs::read_to_string(root.join(format!("{name}.sig"))).unwrap(),version,current)
        };
        let installer=format!("{}_9.9.9_x64-setup.exe",env!("DESKTOP_PRODUCT"));
        assert!(check(&installer,"9.9.9","0.1.0-alpha.11").is_ok(),"genuine newer installer must be accepted");
        assert!(check(&installer,"9.9.10","0.1.0-alpha.11").is_err(),"relabelled version");
        assert!(check(&installer,"9.9.9","9.9.9").is_err(),"same version replay");
        assert!(check(&installer,"9.9.9","10.0.0").is_err(),"downgrade");
        assert!(check("other-product_9.9.9_x64-setup.exe","9.9.9","0.1.0").is_err(),"another product signed with same key");
        assert!(check("unversioned.exe","9.9.9","0.1.0").is_err(),"unsigned version");
        assert!(check("tampered.exe","9.9.9","0.1.0").is_err(),"modified payload");
        assert!(check("wrong-key.exe","9.9.9","0.1.0").is_err(),"untrusted key");
        let signature=std::fs::read_to_string(root.join(format!("{installer}.sig"))).unwrap();
        assert!(verify_installer(std::io::empty(),&signature,"9.9.9","0.1.0").is_err(),"empty/truncated bytes");
        assert!(verify_installer(std::io::empty(),&"x".repeat(8193),"9.9.9","0.1.0").is_err(),"oversize signature");
        assert!(verify_installer(std::io::empty(),&signature,&"9".repeat(129),"0.1.0").is_err(),"oversize version");
    }
}
