//! Calibration: learn each backend's per-call overhead and tokens-per-byte.

use crate::backend::Burner;
use crate::rng::{build_prompt, Rng};
use crate::usage::{accumulate, Report};

/// Linear token model learned at calibration: fresh tokens burned by one call
/// is roughly `overhead + pad_bytes * tokens_per_byte`.
#[derive(Debug, Clone, Copy)]
pub struct Calibration {
    pub overhead: f64,
    pub tokens_per_byte: f64,
}

/// Hard cap on padding per call. It bounds request size and, crucially, keeps
/// a degenerate (near-zero) slope from asking for a usize::MAX-sized string.
pub const MAX_PAD_BYTES: usize = 1_000_000;

impl Calibration {
    /// Pad bytes needed to burn about `tokens` fresh tokens in one call.
    pub fn pad_for(&self, tokens: u64) -> usize {
        let extra = (tokens as f64 - self.overhead).max(0.0);
        if self.tokens_per_byte <= 0.0 {
            0
        } else {
            // Float to usize saturates on overflow, so .min caps it safely.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::Usage;

    #[test]
    fn pad_for_clamps_degenerate_slope() {
        // A near-zero slope must not request a monster string (the overflow bug).
        let cal = Calibration { overhead: 0.0, tokens_per_byte: f64::MIN_POSITIVE };
        assert_eq!(cal.pad_for(60_000), MAX_PAD_BYTES);
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
