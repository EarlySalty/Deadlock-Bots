//! Irreversible database keys for randomly generated bearer capabilities.
//!
//! Callers always hash the presented token, including strings which look like
//! stored keys. A database key must never itself authenticate a request.
use sha2::{Digest, Sha256};

pub const PREFIX: &str = "sha256:";

/// The prefix distinguishes migrated rows from legacy raw tokens.
/// This is for high-entropy capabilities, not user-chosen passwords.
pub fn lookup(raw: &str) -> String {
    format!("{PREFIX}{}", hex::encode(Sha256::digest(raw.as_bytes())))
}

pub fn matches(raw: &str, stored: &str) -> bool {
    let presented = lookup(raw);
    if stored.len() != presented.len() {
        return false;
    }
    presented
        .as_bytes()
        .iter()
        .zip(stored.as_bytes())
        .fold(0u8, |d, (a, b)| d | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_postgresql_sha256_format() {
        assert_eq!(
            lookup("abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    #[test]
    fn database_copy_is_not_a_bearer_token() {
        let raw = "synthetic-capability-with-256-bits-in-production";
        let stored = lookup(raw);
        assert!(matches(raw, &stored));
        assert!(!matches(&stored, &stored));
        assert!(!matches("another-capability", &stored));
        assert!(!matches(raw, raw));
    }
}
