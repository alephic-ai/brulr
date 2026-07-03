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
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Token usage reported by a backend for a single call.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    /// Fresh input that writes the cache — full-price burn. Large prompts land
    /// here rather than in `input_tokens`.
    pub cache_creation_input_tokens: u64,
    pub output_tokens: u64,
    /// Tokens served from cache — cheap (~0.1x) and NOT real burn.
    pub cache_read_input_tokens: u64,
    /// API-equivalent cost of this call, in USD. claude reports it directly;
    /// codex is derived from a price snapshot.
    pub cost_usd: f64,
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
    /// Accumulated API-equivalent cost in USD.
    pub cost_usd: f64,
}

/// Weight applied to cache-read tokens in cost-weighted accounting: cache
/// reads cost roughly a tenth of fresh input.
// ponytail: cache-writes are weighted 1.0 here; real APIs charge ~1.25x —
// bump if that precision ever matters.
pub const CACHE_READ_WEIGHT: f64 = 0.1;

impl Report {
    pub fn processed(&self) -> u64 {
        self.input_tokens + self.cache_creation_input_tokens + self.output_tokens
    }

    /// Every token at face value — the inflated number leaderboards quote.
    pub fn raw_tokens(&self) -> u64 {
        self.processed() + self.cache_read_input_tokens
    }

    /// Honest burn: fresh input/output/cache-writes at full weight, cache
    /// reads discounted to what they actually cost.
    pub fn cost_weighted_tokens(&self) -> f64 {
        self.processed() as f64 + self.cache_read_input_tokens as f64 * CACHE_READ_WEIGHT
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
    report.cost_usd += usage.cost_usd;
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
/// processed or `deadline` passes, continuing from `report` (e.g. the
/// calibration total). The deadline is checked before each call, never
/// mid-call, so a call in flight when it passes completes normally. Each
/// call's pad is sized from `cal` to burn the remaining tokens — capped for
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

/// Known models for the `claude` harness — a static snapshot fetched
/// 2026-07-03 from the provider model API. `--model` is a free pass-through,
/// so newer models still work; this list is only for discovery.
//
// ponytail: hardcoded snapshot, will go stale. To refresh (needs an
// ANTHROPIC_API_KEY — the subscription `claude` CLI does not expose one):
//
//   curl -s https://api.anthropic.com/v1/models \
//     -H "x-api-key: $ANTHROPIC_API_KEY" -H "anthropic-version: 2023-06-01" \
//     | python3 -c 'import json,sys; [print(m["id"]) for m in json.load(sys.stdin)["data"]]'
//
// Paste the resulting ids below (newest first) and bump the date above.
pub const CLAUDE_MODELS: &[&str] = &[
    "claude-sonnet-5",
    "claude-fable-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-sonnet-4-6",
    "claude-opus-4-6",
    "claude-opus-4-5-20251101",
    "claude-haiku-4-5-20251001",
    "claude-sonnet-4-5-20250929",
    "claude-opus-4-1-20250805",
];

/// Codex-family models for the `codex` harness (OpenAI). Same snapshot/caveat
/// as [`CLAUDE_MODELS`]; the OpenAI model API also lists many non-coding
/// models (audio, image, embeddings) that codex can't use — those are omitted.
//
// To refresh (needs an OPENAI_API_KEY), keep only the coding models — the
// `*-codex` ids (and general `gpt-*`/`o*` reasoning models if desired):
//
//   curl -s https://api.openai.com/v1/models \
//     -H "Authorization: Bearer $OPENAI_API_KEY" \
//     | python3 -c 'import json,sys; [print(m["id"]) for m in json.load(sys.stdin)["data"] if "codex" in m["id"]]'
//
// Paste the resulting ids below (newest first) and bump the date above.
pub const CODEX_MODELS: &[&str] = &[
    "gpt-5.3-codex",
    "gpt-5.2-codex",
    "gpt-5.1-codex-max",
    "gpt-5.1-codex-mini",
    "gpt-5.1-codex",
    "gpt-5-codex",
];

/// Reasoning-effort levels the `claude` harness accepts.
// To update: `claude --help` lists the values on the `--effort <level>` line.
pub const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Reasoning-effort levels the `codex` harness accepts (config
/// `model_reasoning_effort`).
// To update: see codex's `model_reasoning_effort` config documentation.
pub const CODEX_EFFORTS: &[&str] = &["minimal", "low", "medium", "high"];

/// The real backend: shells out to the `claude` CLI in one-shot mode.
pub struct ClaudeBurner {
    pub model: Option<String>,
    pub effort: Option<String>,
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
        if let Some(e) = &self.effort {
            cmd.args(["--effort", e]);
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

/// The `codex` backend: shells out to `codex exec` in non-interactive mode.
pub struct CodexBurner {
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl Burner for CodexBurner {
    fn run(&mut self, prompt: &str) -> Result<Usage, String> {
        let mut cmd = Command::new("codex");
        // ponytail: these flags are version-sensitive (codex 0.142.x). `-c
        // approval_policy=never` is the unattended shim; read-only sandbox
        // keeps a burn prompt from touching the machine.
        cmd.args([
            "exec",
            "--json",
            "--skip-git-repo-check",
            "--ephemeral",
            "-s",
            "read-only",
            "-c",
            "approval_policy=never",
        ]);
        if let Some(m) = &self.model {
            cmd.args(["-m", m]);
        }
        if let Some(e) = &self.effort {
            cmd.args(["-c", &format!("model_reasoning_effort={e}")]);
        }
        cmd.arg(prompt);
        let out = cmd.output().map_err(|e| format!("spawn codex: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "codex exited {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let mut usage = parse_codex_usage(&out.stdout)?;
        usage.cost_usd = codex_cost(self.model.as_deref(), &usage);
        Ok(usage)
    }
}

/// Sum token usage from codex's JSONL `turn.completed` events. codex reports
/// `input_tokens` inclusive of `cached_input_tokens`, so fresh input is the
/// difference; reasoning tokens count as output; there is no cache-creation.
pub fn parse_codex_usage(stdout: &[u8]) -> Result<Usage, String> {
    let mut input = 0u64;
    let mut cached = 0u64;
    let mut output = 0u64;
    let mut seen = false;
    for line in stdout.split(|&b| b == b'\n') {
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if v["type"] != "turn.completed" {
            continue;
        }
        seen = true;
        let u = &v["usage"];
        input += u["input_tokens"].as_u64().unwrap_or(0);
        cached += u["cached_input_tokens"].as_u64().unwrap_or(0);
        output += u["output_tokens"].as_u64().unwrap_or(0)
            + u["reasoning_output_tokens"].as_u64().unwrap_or(0);
    }
    if !seen {
        return Err("no turn.completed usage event in codex output".into());
    }
    Ok(Usage {
        input_tokens: input.saturating_sub(cached),
        cache_creation_input_tokens: 0,
        output_tokens: output,
        cache_read_input_tokens: cached,
        cost_usd: 0.0, // filled in by CodexBurner from CODEX_PRICES
    })
}

/// Codex price snapshot: (model, input, cached-input, output) in USD per 1M
/// tokens. codex does not report cost, so dollar output is derived from this.
//
// Verified 2026-07-04 against OpenAI's pricing page and OpenRouter. gpt-5-codex
// has no current listing (legacy) and is assumed to match the 5.1-codex tier.
// To refresh, see https://developers.openai.com/api/docs/pricing. First entry
// is the assumed default when `--model` is omitted or unknown.
pub const CODEX_PRICES: &[(&str, f64, f64, f64)] = &[
    ("gpt-5.3-codex", 1.75, 0.175, 14.0),
    ("gpt-5.2-codex", 1.75, 0.175, 14.0),
    ("gpt-5.1-codex-max", 1.25, 0.125, 10.0),
    ("gpt-5.1-codex-mini", 0.25, 0.025, 2.0),
    ("gpt-5.1-codex", 1.25, 0.125, 10.0),
    ("gpt-5-codex", 1.25, 0.125, 10.0), // legacy: inferred, not listed
];

/// USD cost of `usage` for a codex `model` (None/unknown → the default, the
/// first `CODEX_PRICES` entry).
pub fn codex_cost(model: Option<&str>, usage: &Usage) -> f64 {
    let (_, inp, cached, out) = model
        .and_then(|m| CODEX_PRICES.iter().find(|(name, ..)| *name == m).copied())
        .unwrap_or(CODEX_PRICES[0]);
    (usage.input_tokens as f64 * inp
        + usage.cache_read_input_tokens as f64 * cached
        + usage.output_tokens as f64 * out)
        / 1_000_000.0
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
        // `total_cost_usd` sits at the top level of the result, not under usage.
        cost_usd: v["total_cost_usd"].as_f64().unwrap_or(0.0),
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
        let json = br#"{"total_cost_usd":0.0123,"usage":{"input_tokens":2000,"cache_creation_input_tokens":23000,"output_tokens":3,"cache_read_input_tokens":40}}"#;
        let u = parse_usage(json).unwrap();
        assert_eq!(u.input_tokens, 2000);
        assert_eq!(u.cache_creation_input_tokens, 23000);
        assert_eq!(u.output_tokens, 3);
        assert_eq!(u.cache_read_input_tokens, 40);
        assert!((u.cost_usd - 0.0123).abs() < 1e-9); // top-level total_cost_usd
        // Cache-creation counts as burn; cache-read does not.
        assert_eq!(u.processed(), 25003);
    }

    #[test]
    fn codex_cost_uses_price_table() {
        let usage = Usage {
            input_tokens: 1_000_000,
            cache_read_input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            ..Default::default()
        };
        let (_, inp, cached, out) = CODEX_PRICES[0];
        let expected = inp + cached + out; // 1M of each ÷ 1M
        assert!((codex_cost(Some(CODEX_PRICES[0].0), &usage) - expected).abs() < 1e-9);
        // Unknown/None model falls back to the first entry.
        assert!((codex_cost(None, &usage) - expected).abs() < 1e-9);
        assert!((codex_cost(Some("nope"), &usage) - expected).abs() < 1e-9);
    }

    #[test]
    fn burn_stops_at_dollar_target() {
        let mut rng = Rng::from_seed(1);
        let cal = Calibration { overhead: 0.0, tokens_per_byte: 1.0 };
        let mut b = FakeBurner {
            per_call: Usage { input_tokens: 1000, cost_usd: 0.10, ..Default::default() },
        };
        // $0.25 target, $0.10/call → 3 calls (0.30 ≥ 0.25).
        let r = burn(u64::MAX, None, Some(0.25), &cal, Report::default(), &mut rng, &mut b, &mut |_| {})
            .unwrap();
        assert_eq!(r.calls, 3);
        assert!((r.cost_usd - 0.30).abs() < 1e-9);
    }

    #[test]
    fn parse_codex_usage_maps_fields() {
        // Real codex 0.142 shape; input_tokens is inclusive of cached.
        let jsonl = concat!(
            "{\"type\":\"item.completed\",\"item\":{\"text\":\"ok\"}}\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":14876,\"cached_input_tokens\":9600,\"output_tokens\":21,\"reasoning_output_tokens\":14}}\n",
        );
        let u = parse_codex_usage(jsonl.as_bytes()).unwrap();
        assert_eq!(u.input_tokens, 14876 - 9600); // fresh input only
        assert_eq!(u.cache_read_input_tokens, 9600);
        assert_eq!(u.output_tokens, 21 + 14); // reasoning counts as output
        assert_eq!(u.cache_creation_input_tokens, 0);
        assert_eq!(u.processed(), 5276 + 35);
    }

    #[test]
    fn parse_codex_usage_errs_without_event() {
        assert!(parse_codex_usage(b"{\"type\":\"item.completed\"}\n").is_err());
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
        let r = burn(2500, None, None, &cal, Report::default(), &mut rng, &mut b, &mut |_| ticks += 1).unwrap();
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
    fn accounting_separates_raw_from_cost_weighted() {
        let r = Report {
            calls: 1,
            input_tokens: 1000,
            cache_creation_input_tokens: 500,
            output_tokens: 100,
            cache_read_input_tokens: 900,
            ..Default::default()
        };
        assert_eq!(r.processed(), 1600); // excludes cache reads
        assert_eq!(r.raw_tokens(), 2500); // everything at face value
        assert!((r.cost_weighted_tokens() - 1690.0).abs() < 1e-9); // 1600 + 0.1*900
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
