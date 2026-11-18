//! Opaque 256-bit credentials shared by authentication and host enrollment.
use sha2::{Digest, Sha256};

pub fn fingerprint(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

pub fn secret() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill(&mut bytes[..]);
    hex::encode(bytes)
}
