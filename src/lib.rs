//! brülr — burn AI tokens on purpose.
//!
//! The overhead strategy: send many calls, each padded with uncacheable
//! random bytes. Input tokens ingest far faster than output generates, so
//! padding maximizes burn per call at minimal latency. A fresh random pad
//! per call defeats prefix caching (which dies on the first divergent token).
//!
//! The loop is backend-agnostic via [`Burner`]; [`ClaudeBurner`] is the real
//! backend and tests use a fake one.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Token usage reported by a backend for a single call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    /// Fresh input that writes the cache — full-price burn. Large prompts land
    /// here rather than in `input_tokens`.
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
    /// Tokens served from cache — cheap (~0.1x) and NOT real burn.
    pub cache_read_input_tokens: u64,
}

impl Usage {
    /// Fresh tokens actually processed — what counts toward a burn target.
    pub fn processed(&self) -> u64 {
        self.input_tokens + self.cache_creation_input_tokens + self.output_tokens
    }
}

/// A backend that executes one prompt and reports its token usage.
pub trait Burner {
    fn run(&mut self, prompt: &str) -> Result<Usage, String>;
}

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

/// Accumulated result of a burn run.
#[derive(Debug, Default, Clone, Copy)]
pub struct Report {
    pub calls: u64,
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
}

impl Report {
    pub fn processed(&self) -> u64 {
        self.input_tokens + self.cache_creation_input_tokens + self.output_tokens
    }

    /// Fraction of input served from cache. High means the padding is being
    /// cached and the burn is not real — entropy should keep this near zero.
    pub fn cache_hit_ratio(&self) -> f64 {
        let total = self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens;
        if total == 0 {
            0.0
        } else {
            self.cache_read_input_tokens as f64 / total as f64
        }
    }
}

fn accumulate(report: &mut Report, usage: &Usage) {
    report.calls += 1;
    report.input_tokens += usage.input_tokens;
    report.cache_creation_input_tokens += usage.cache_creation_input_tokens;
    report.output_tokens += usage.output_tokens;
    report.cache_read_input_tokens += usage.cache_read_input_tokens;
}

/// Linear token model learned at calibration: fresh tokens burned by one call
/// ≈ `overhead + pad_bytes * tokens_per_byte`.
#[derive(Debug, Clone, Copy)]
pub struct Calibration {
    pub overhead: f64,
    pub tokens_per_byte: f64,
}

/// Hard cap on padding per call — bounds request size and, crucially, keeps a
/// degenerate (near-zero) slope from asking for a usize::MAX-sized string.
pub const MAX_PAD_BYTES: usize = 1_000_000;

impl Calibration {
    /// Pad bytes needed to burn about `tokens` fresh tokens in one call.
    pub fn pad_for(&self, tokens: u64) -> usize {
        let extra = (tokens as f64 - self.overhead).max(0.0);
        if self.tokens_per_byte <= 0.0 {
            0
        } else {
            // Float→usize saturates on overflow, so .min caps it safely.
            (extra / self.tokens_per_byte).min(MAX_PAD_BYTES as f64) as usize
        }
    }
}

/// Number of probe calls [`calibrate`] makes.
pub const PROBES: u64 = 2;

/// Learn the token model with two probe calls: an empty pad measures the
/// fixed per-call overhead, a sized pad measures tokens-per-byte. The probes
/// burn real tokens, so their usage is returned to fold into the burn total.
/// `on_progress` fires after each probe.
pub fn calibrate(
    rng: &mut Rng,
    burner: &mut dyn Burner,
    on_progress: &mut dyn FnMut(&Report),
) -> Result<(Calibration, Report), String> {
    const PROBE_BYTES: usize = 20_000;
    let mut report = Report::default();

    let empty = build_prompt(rng, 0);
    let u0 = burner.run(&empty)?;
    accumulate(&mut report, &u0);
    on_progress(&report);

    let sized = build_prompt(rng, PROBE_BYTES);
    let u1 = burner.run(&sized)?;
    accumulate(&mut report, &u1);
    on_progress(&report);

    let overhead = u0.processed() as f64;
    let tokens_per_byte =
        ((u1.processed() as f64 - overhead) / PROBE_BYTES as f64).max(f64::MIN_POSITIVE);
    Ok((Calibration { overhead, tokens_per_byte }, report))
}

/// Run the overhead burn loop until `target_tokens` fresh tokens are
/// processed, continuing from `report` (e.g. the calibration total). Each
/// call's pad is sized from `cal` to burn the remaining tokens — capped for
/// request-size safety, and trimmed on the final call to avoid overshoot.
/// `on_progress` fires after every call; the library stays clock-free, so
/// timing/ETA is the caller's concern.
pub fn burn(
    target_tokens: u64,
    cal: &Calibration,
    mut report: Report,
    rng: &mut Rng,
    burner: &mut dyn Burner,
    on_progress: &mut dyn FnMut(&Report),
) -> Result<Report, String> {
    // ponytail: fixed request-size rail; probe to the backend's real ceiling
    // if minimizing round trips ever matters more than simplicity.
    const MAX_TOKENS_PER_CALL: u64 = 60_000;
    while report.processed() < target_tokens {
        let want = (target_tokens - report.processed()).min(MAX_TOKENS_PER_CALL);
        let prompt = build_prompt(rng, cal.pad_for(want));
        let usage = burner.run(&prompt)?;
        accumulate(&mut report, &usage);
        on_progress(&report);
    }
    Ok(report)
}

/// The real backend: shells out to the `claude` CLI in one-shot mode.
pub struct ClaudeBurner {
    pub model: Option<String>,
}

impl Burner for ClaudeBurner {
    fn run(&mut self, prompt: &str) -> Result<Usage, String> {
        let mut cmd = Command::new("claude");
        cmd.args([
            "-p",
            "--output-format",
            "json",
            "--no-session-persistence",
            "--tools",
            "",
        ]);
        if let Some(m) = &self.model {
            cmd.args(["--model", m]);
        }
        cmd.arg("--").arg(prompt); // `--` matters: --tools is variadic
        let out = cmd.output().map_err(|e| format!("spawn claude: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "claude exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        parse_usage(&out.stdout)
    }
}

/// Extract the `usage` block from claude's JSON result.
pub fn parse_usage(stdout: &[u8]) -> Result<Usage, String> {
    let v: serde_json::Value =
        serde_json::from_slice(stdout).map_err(|e| format!("parse json: {e}"))?;
    let u = &v["usage"];
    Ok(Usage {
        input_tokens: u["input_tokens"].as_u64().unwrap_or(0),
        cache_creation_input_tokens: u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
        output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
        cache_read_input_tokens: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
    })
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

    #[test]
    fn parse_usage_reads_fields() {
        let json = br#"{"usage":{"input_tokens":2000,"cache_creation_input_tokens":23000,"output_tokens":3,"cache_read_input_tokens":40}}"#;
        let u = parse_usage(json).unwrap();
        assert_eq!(u.input_tokens, 2000);
        assert_eq!(u.cache_creation_input_tokens, 23000);
        assert_eq!(u.output_tokens, 3);
        assert_eq!(u.cache_read_input_tokens, 40);
        // Cache-creation counts as burn; cache-read does not.
        assert_eq!(u.processed(), 25003);
    }

    #[test]
    fn pad_for_clamps_degenerate_slope() {
        // A near-zero slope must not request a monster string (the overflow bug).
        let cal = Calibration { overhead: 0.0, tokens_per_byte: f64::MIN_POSITIVE };
        assert_eq!(cal.pad_for(60_000), MAX_PAD_BYTES);
    }

    // Fake backend: fixed usage per call, no `claude` needed.
    struct FakeBurner {
        per_call: Usage,
    }
    impl Burner for FakeBurner {
        fn run(&mut self, _prompt: &str) -> Result<Usage, String> {
            Ok(self.per_call)
        }
    }

    #[test]
    fn burn_loops_until_target_reached() {
        let mut rng = Rng::from_seed(1);
        let mut b = FakeBurner {
            per_call: Usage {
                input_tokens: 1000,
                cache_read_input_tokens: 500,
                ..Default::default()
            },
        };
        let cal = Calibration { overhead: 0.0, tokens_per_byte: 1.0 };
        let mut ticks = 0;
        let r = burn(2500, &cal, Report::default(), &mut rng, &mut b, &mut |_| ticks += 1).unwrap();
        assert_eq!(ticks, 3); // progress fires once per call
        assert_eq!(r.calls, 3); // 1000*3 = 3000 >= 2500
        assert_eq!(r.processed(), 3000);
        assert_eq!(r.cache_read_input_tokens, 1500); // not counted toward target
    }

    #[test]
    fn cache_hit_ratio_flags_absorbed_padding() {
        let r = Report {
            calls: 1,
            input_tokens: 10,
            cache_read_input_tokens: 90,
            ..Default::default()
        };
        assert!((r.cache_hit_ratio() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn burn_propagates_backend_error() {
        struct Failing;
        impl Burner for Failing {
            fn run(&mut self, _: &str) -> Result<Usage, String> {
                Err("boom".into())
            }
        }
        let mut rng = Rng::from_seed(1);
        let cal = Calibration { overhead: 0.0, tokens_per_byte: 1.0 };
        assert!(burn(100, &cal, Report::default(), &mut rng, &mut Failing, &mut |_| {}).is_err());
    }

    // Backend whose usage scales with prompt length, so calibration can
    // recover a real tokens-per-byte slope.
    struct LinearFake;
    impl Burner for LinearFake {
        fn run(&mut self, prompt: &str) -> Result<Usage, String> {
            Ok(Usage {
                input_tokens: 5000 + prompt.len() as u64 / 2,
                ..Default::default()
            })
        }
    }

    #[test]
    fn calibrate_recovers_overhead_and_slope() {
        let mut rng = Rng::from_seed(1);
        let mut ticks = 0;
        let (cal, report) = calibrate(&mut rng, &mut LinearFake, &mut |_| ticks += 1).unwrap();
        assert_eq!(report.calls, 2); // two probe calls
        assert_eq!(ticks, 2); // progress fires once per probe
        assert!((cal.tokens_per_byte - 0.5).abs() < 0.01);
        assert!(cal.overhead >= 5000.0 && cal.overhead < 5050.0);
        assert!(cal.pad_for(10_000) > 0);
    }
}
