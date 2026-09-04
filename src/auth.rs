use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use argon2::password_hash::{PasswordHash, SaltString};
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub fn valid_password(value: &str) -> bool {
    (15..=128).contains(&value.chars().count()) && value.len() <= 512
}

pub fn hash_password(value: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| anyhow::anyhow!("密码摘要生成失败"))
}

pub fn valid_credential(encoded: &str) -> bool {
    if encoded.starts_with("$argon2id$") {
        PasswordHash::new(encoded).is_ok_and(|hash| {
            hash.params.get_decimal("m") == Some(19456)
                && hash.params.get_decimal("t") == Some(2)
                && hash.params.get_decimal("p") == Some(1)
                && hash.hash.is_some()
                && hash.salt.is_some()
        })
    } else {
        URL_SAFE_NO_PAD
            .decode(encoded)
            .is_ok_and(|hash| hash.len() == 32)
    }
}

pub fn verify_password(encoded: &str, provided: &str) -> bool {
    if provided.len() > 512 || !valid_credential(encoded) {
        return false;
    }
    if encoded.starts_with("$argon2id$") {
        PasswordHash::new(encoded).is_ok_and(|hash| {
            Argon2::default()
                .verify_password(provided.as_bytes(), &hash)
                .is_ok()
        })
    } else {
        let expected = URL_SAFE_NO_PAD.decode(encoded).unwrap_or_default();
        let actual: [u8; 32] = Sha256::digest(provided.as_bytes()).into();
        bool::from(expected.as_slice().ct_eq(&actual))
    }
}

pub(crate) struct AdminAuth {
    pub credential: String,
    pub sessions: HashMap<String, Instant>,
    attempts: u8,
    window: Instant,
}

impl AdminAuth {
    pub fn new(credential: String) -> Result<Self> {
        if !valid_credential(&credential) {
            bail!("管理员密码摘要损坏");
        }
        Ok(Self {
            credential,
            sessions: HashMap::new(),
            attempts: 0,
            window: Instant::now(),
        })
    }

    // One administrator-wide budget cannot be bypassed by spoofing proxy headers.
    // A short fixed window avoids permanent account lockout and unbounded IP maps.
    pub fn allow_attempt(&mut self, now: Instant) -> bool {
        if now.duration_since(self.window) >= Duration::from_secs(60) {
            self.window = now;
            self.attempts = 0;
        }
        if self.attempts >= 5 {
            return false;
        }
        self.attempts += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_use_salted_argon2id_and_legacy_secrets_still_work() {
        let value = "a long test password";
        let first = hash_password(value).unwrap();
        let second = hash_password(value).unwrap();
        assert!(first.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert_ne!(first, second);
        assert!(verify_password(&first, value));
        assert!(!verify_password(&first, "incorrect"));
        let legacy = URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()));
        assert!(verify_password(&legacy, value));
        assert!(!verify_password(&legacy, "incorrect"));
        assert!(!verify_password("broken", value));
    }

    #[test]
    fn password_rules_preserve_spaces_unicode_and_reasonable_bounds() {
        assert!(!valid_password("short"));
        assert!(valid_password("a long pass phrase"));
        assert!(valid_password(&"字".repeat(15)));
        assert!(!valid_password(&"a".repeat(129)));
    }

    #[test]
    fn attempt_budget_is_bounded_and_expires() {
        let mut auth = AdminAuth::new(URL_SAFE_NO_PAD.encode([1_u8; 32])).unwrap();
        let now = auth.window;
        for _ in 0..5 {
            assert!(auth.allow_attempt(now));
        }
        assert!(!auth.allow_attempt(now + Duration::from_secs(59)));
        assert!(auth.allow_attempt(now + Duration::from_secs(60)));
    }
}
