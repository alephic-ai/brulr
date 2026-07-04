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

/// Stems the burn tasks are built from; exposed so tests can recognize a
/// generated instruction.
pub const TASK_STEMS: [&str; 4] = [
    "Write the integers from",
    "Print the multiplication table of",
    "Convert each of these hex values to decimal and binary",
    "List the numbers from",
];

/// Pick a task template and fill in rng-drawn parameters. Each task is
/// self-contained (works with an empty pad, so the calibration probe is
/// unaffected) and trivially satisfiable, sized for ~500-2000 output tokens.
// ponytail: bounds assume harness output caps well above 2k tokens; tune the
// ranges if a backend starts truncating replies.
fn build_task(rng: &mut Rng) -> String {
    // Single-turn matters: tool-use turns re-read the whole padded context
    // from cache, ballooning cache reads and tripping the absorbed-padding
    // warning.
    let task = match rng.next_u64() % 4 {
        0 => {
            let a = 1000 + rng.next_u64() % 9000;
            let n = 150 + rng.next_u64() % 200;
            format!(
                "{} {a} to {} in English words, one per line.",
                TASK_STEMS[0],
                a + n
            )
        }
        1 => {
            let x = 10 + rng.next_u64() % 90;
            let n = 40 + rng.next_u64() % 40;
            format!(
                "{} {x} from 1 to {n}, showing each product's full arithmetic.",
                TASK_STEMS[1]
            )
        }
        2 => {
            let k = 10 + rng.next_u64() % 10;
            let words: Vec<String> = (0..k).map(|_| format!("{:016x}", rng.next_u64())).collect();
            format!("{}, showing your work: {}", TASK_STEMS[2], words.join(" "))
        }
        _ => {
            let a = 1000 + rng.next_u64() % 9000;
            let n = 150 + rng.next_u64() % 150;
            format!(
                "{} {a} down to {}, each with its digit sum.",
                TASK_STEMS[3],
                a - n
            )
        }
    };
    format!("{task} Answer directly in one reply; do not run commands or use tools.")
}

/// `pad_bytes` of random hex.
fn pad(rng: &mut Rng, pad_bytes: usize) -> String {
    let mut s = String::with_capacity(pad_bytes + 128);
    while s.len() < pad_bytes {
        s.push_str(&format!("{:016x}", rng.next_u64()));
    }
    s.truncate(pad_bytes); // hex is ASCII, every index is a char boundary
    s
}

/// Build a burn prompt: `pad_bytes` of random hex followed by a rotating
/// rng-parameterized task that burns a bounded amount of output tokens.
pub fn build_prompt(rng: &mut Rng, pad_bytes: usize) -> String {
    let mut s = pad(rng, pad_bytes);
    s.push('\n');
    s.push_str(&build_task(rng));
    s
}

/// Build a calibration probe: padding plus a fixed near-zero-output reply.
/// Probes must NOT carry a rotating task — its output variance (especially
/// codex reasoning tokens) can swamp the pad signal and drive the two-point
/// fit to a degenerate slope.
pub fn build_probe(rng: &mut Rng, pad_bytes: usize) -> String {
    let mut s = pad(rng, pad_bytes);
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
        let tail = &p[1000..];
        assert!(TASK_STEMS.iter().any(|s| tail.contains(s)));
    }

    #[test]
    fn probe_keeps_output_minimal() {
        let mut rng = Rng::from_seed(1);
        let p = build_probe(&mut rng, 1000);
        assert!(p.len() >= 1000);
        assert!(p.ends_with("Reply with exactly: ok"));
    }

    #[test]
    fn tasks_vary_across_calls_and_are_seed_deterministic() {
        let mut rng = Rng::from_seed(9);
        let a = build_task(&mut rng);
        let b = build_task(&mut rng);
        assert_ne!(a, b); // parameters (at least) diverge call to call
        let mut rng2 = Rng::from_seed(9);
        assert_eq!(a, build_task(&mut rng2)); // same seed, same task
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
