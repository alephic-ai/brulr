//! The overhead burn loop.

use std::time::Instant;

use crate::backend::Burner;
use crate::calibrate::Calibration;
use crate::rng::{build_prompt, Rng};
use crate::usage::{accumulate, Report};

/// Run the overhead burn loop until `target_tokens` fresh tokens are
/// processed or `deadline` passes, continuing from `report` (e.g. the
/// calibration total). The deadline is checked before each call, never
/// mid-call, so a call in flight when it passes completes normally. Each
/// call's pad is sized from `cal` to burn the remaining tokens, capped for
/// request-size safety, and trimmed on the final call to avoid overshoot.
/// `on_progress` fires after every call; rate/ETA math is the caller's
/// concern.
pub fn burn(
    target_tokens: u64,
    deadline: Option<Instant>,
    target_usd: Option<f64>,
    cal: &Calibration,
    mut report: Report,
    rng: &mut Rng,
    burner: &mut dyn Burner,
    on_progress: &mut dyn FnMut(&Report),
) -> Result<Report, String> {
    // ponytail: fixed request-size rail; probe to the backend's real ceiling
    // if minimizing round trips ever matters more than simplicity.
    const MAX_TOKENS_PER_CALL: u64 = 60_000;
    while report.processed() < target_tokens
        && deadline.is_none_or(|d| Instant::now() < d)
        && target_usd.is_none_or(|u| report.cost_usd < u)
    {
        let want = (target_tokens - report.processed()).min(MAX_TOKENS_PER_CALL);
        let prompt = build_prompt(rng, cal.pad_for(want));
        let usage = burner.run(&prompt)?;
        accumulate(&mut report, &usage);
        on_progress(&report);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::Usage;

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
        let r = burn(2500, None, None, &cal, Report::default(), &mut rng, &mut b, &mut |_| ticks += 1).unwrap();
        assert_eq!(ticks, 3); // progress fires once per call
        assert_eq!(r.calls, 3); // 1000*3 = 3000 >= 2500
        assert_eq!(r.processed(), 3000);
        assert_eq!(r.cache_read_input_tokens, 1500); // not counted toward target
    }

    #[test]
    fn burn_stops_at_dollar_target() {
        let mut rng = Rng::from_seed(1);
        let cal = Calibration { overhead: 0.0, tokens_per_byte: 1.0 };
        let mut b = FakeBurner {
            per_call: Usage { input_tokens: 1000, cost_usd: 0.10, ..Default::default() },
        };
        // $0.25 target at $0.10/call gives 3 calls (0.30 >= 0.25).
        let r = burn(u64::MAX, None, Some(0.25), &cal, Report::default(), &mut rng, &mut b, &mut |_| {})
            .unwrap();
        assert_eq!(r.calls, 3);
        assert!((r.cost_usd - 0.30).abs() < 1e-9);
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
        assert!(burn(100, None, None, &cal, Report::default(), &mut rng, &mut Failing, &mut |_| {}).is_err());
    }

    #[test]
    fn expired_deadline_stops_before_first_call() {
        let mut rng = Rng::from_seed(1);
        let cal = Calibration { overhead: 0.0, tokens_per_byte: 1.0 };
        let mut b = FakeBurner { per_call: Usage { input_tokens: 1000, ..Default::default() } };
        let past = Instant::now() - std::time::Duration::from_secs(1);
        let r = burn(u64::MAX, Some(past), None, &cal, Report::default(), &mut rng, &mut b, &mut |_| {}).unwrap();
        assert_eq!(r.calls, 0);
    }

    #[test]
    fn future_deadline_still_honors_token_target() {
        let mut rng = Rng::from_seed(1);
        let cal = Calibration { overhead: 0.0, tokens_per_byte: 1.0 };
        let mut b = FakeBurner { per_call: Usage { input_tokens: 1000, ..Default::default() } };
        let future = Instant::now() + std::time::Duration::from_secs(3600);
        let r = burn(2500, Some(future), None, &cal, Report::default(), &mut rng, &mut b, &mut |_| {}).unwrap();
        assert_eq!(r.calls, 3);
    }
}
