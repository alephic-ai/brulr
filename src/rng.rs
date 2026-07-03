//! A small PRNG and the random-padding prompt builder.

use std::time::{SystemTime, UNIX_EPOCH};

/// Small non-crypto PRNG (xorshift64).
// ponytail: cache-defeat only needs divergent bytes, not crypto entropy, so
// this avoids a getrandom dependency. Swap in getrandom if crypto-grade is
// ever needed.
pub struct Rng(u64);

impl Rng {
    pub fn from_seed(seed: u64) -> Self {
        Rng(seed | 1) // avoid xorshift's zero fixed-point
    }

    pub fn from_entropy() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        Rng::from_seed(nanos ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
}

/// Build a burn prompt: `pad_bytes` of random hex followed by a minimal
/// instruction that keeps output near zero.
pub fn build_prompt(rng: &mut Rng, pad_bytes: usize) -> String {
    let mut s = String::with_capacity(pad_bytes + 32);
    while s.len() < pad_bytes {
        s.push_str(&format!("{:016x}", rng.next_u64()));
    }
    s.truncate(pad_bytes); // hex is ASCII, every index is a char boundary
    s.push_str("\nReply with exactly: ok");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_per_seed() {
        let mut a = Rng::from_seed(42);
        let mut b = Rng::from_seed(42);
        assert_eq!(a.next_u64(), b.next_u64());
        let mut c = Rng::from_seed(43);
        assert_ne!(a.next_u64(), c.next_u64());
    }

    #[test]
    fn prompt_meets_pad_size_and_carries_instruction() {
        let mut rng = Rng::from_seed(1);
        let p = build_prompt(&mut rng, 1000);
        assert!(p.len() >= 1000);
        assert!(p.ends_with("Reply with exactly: ok"));
    }

    #[test]
    fn consecutive_prompts_diverge_for_cache_defeat() {
        let mut rng = Rng::from_seed(7);
        let a = build_prompt(&mut rng, 256);
        let b = build_prompt(&mut rng, 256);
        assert_ne!(a, b);
        // Prefix must differ or prefix-caching survives.
        assert_ne!(&a[..16], &b[..16]);
    }
}
