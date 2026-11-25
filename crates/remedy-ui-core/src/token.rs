//! Secrets for local connections (agents, remote devtools): tokens no other process can
//! guess, and a comparison that takes the same time wherever two tokens differ.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

/// The length of a [`random_token`]: 128 bits as lowercase hex.
pub const TOKEN_LEN: usize = 32;

/// A secret no other process can guess: 128 bits from the system's randomness (the standard
/// library's hasher seeds, which a thread draws from the OS once and varies for each hasher,
/// so each half has its own) and the time, as [`TOKEN_LEN`] lowercase hex digits.
pub fn random_token() -> String {
    let mut out = String::with_capacity(TOKEN_LEN);
    for i in 0..2u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(
            crate::time::SystemTime::now()
                .duration_since(crate::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// True when `a` and `b` are the same, taking the same time wherever they first differ, so
/// timing a refusal tells a guesser nothing about how close the guess was.
pub fn same_secret(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = u8::from(a.len() != b.len());
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_hex_of_the_stated_length_and_differ() {
        let a = random_token();
        let b = random_token();
        assert_eq!(a.len(), TOKEN_LEN);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, b);
    }

    #[test]
    fn secrets_compare_exactly() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("abc", "ab"));
        assert!(!same_secret("", "a"));
        assert!(same_secret("", ""));
    }
}
