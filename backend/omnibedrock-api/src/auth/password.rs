//! Password hashing and verification: Argon2id hashes for new and upgraded
//! passwords, with a compatibility path that verifies legacy Python
//! PBKDF2-HMAC-SHA256 hashes so those accounts upgrade at login.

use argon2::{
    Argon2, PasswordHasher, PasswordVerifier,
    password_hash::{PasswordHash, SaltString, rand_core::OsRng},
};
use sha2::Sha256;
use subtle::ConstantTimeEq;

pub(super) fn hash_password(password: &str) -> anyhow::Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("password hash: {e}"))
}
pub(super) fn verify_password(password: &str, stored: &str) -> bool {
    if stored.starts_with("$argon2") {
        return PasswordHash::new(stored).is_ok_and(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        });
    }
    // Existing Python PBKDF2-HMAC-SHA256 accounts upgrade to Argon2id at login.
    if let Some((salt, key)) = stored.split_once(':')
        && let (Ok(salt), Ok(key)) = (hex::decode(salt), hex::decode(key))
    {
        let expected = pbkdf2::pbkdf2_hmac_array::<Sha256, 32>(password.as_bytes(), &salt, 100_000);
        return bool::from(expected.as_slice().ct_eq(&key));
    }
    false
}
