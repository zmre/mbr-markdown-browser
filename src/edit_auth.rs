//! Authentication and hashing helpers for the in-browser editing endpoints.
//!
//! Editing is gated by a pre-shared bearer token. The token is never stored;
//! only an Argon2 PHC hash of it lives in `.mbr/config.toml`
//! (`edit_token_hash`). `verify_token` compares a presented token against that
//! hash in constant time (Argon2's verifier).
//!
//! [`content_hash`] provides the SHA-256 used for optimistic-concurrency
//! checks on the raw-markdown / save endpoints.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use sha2::{Digest, Sha256};

/// Hashes a token/password into an Argon2 PHC string (includes a random salt).
///
/// The returned string is what belongs in `edit_token_hash`.
pub fn hash_token(token: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default().hash_password(token.as_bytes(), &salt)?;
    Ok(hash.to_string())
}

/// Verifies a presented token against a stored Argon2 PHC hash.
///
/// Returns `false` for any parse or verification failure (never panics), so a
/// malformed configured hash simply denies access rather than crashing.
pub fn verify_token(hash: &str, token: &str) -> bool {
    match PasswordHash::new(hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(token.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Generates a random 32-byte token, hex-encoded (64 chars).
///
/// Used by `--generate-edit-token` when the user does not supply a password.
pub fn generate_token() -> String {
    use argon2::password_hash::rand_core::RngCore;
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    to_hex(&bytes)
}

/// SHA-256 of the given bytes, hex-encoded. Used for optimistic-concurrency
/// hashes exchanged with the editor client (`X-MBR-Content-Hash`).
pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    to_hex(&digest)
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_roundtrip() {
        let hash = hash_token("correct horse battery staple").unwrap();
        assert!(verify_token(&hash, "correct horse battery staple"));
    }

    #[test]
    fn verify_rejects_wrong_token() {
        let hash = hash_token("s3cret").unwrap();
        assert!(!verify_token(&hash, "not-the-token"));
    }

    #[test]
    fn verify_rejects_malformed_hash() {
        assert!(!verify_token("not-a-phc-string", "whatever"));
        assert!(!verify_token("", "whatever"));
    }

    #[test]
    fn generated_tokens_are_hex_and_unique() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn content_hash_is_stable_and_sensitive() {
        assert_eq!(content_hash(b"hello"), content_hash(b"hello"));
        assert_ne!(content_hash(b"hello"), content_hash(b"hello!"));
        // Known SHA-256 of the empty input.
        assert_eq!(
            content_hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
