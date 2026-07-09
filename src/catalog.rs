//! Static snapshots of harness models, effort levels, and codex/grok prices.

/// Known models for the `claude` harness: a static snapshot fetched
/// 2026-07-03 from the provider model API. `--model` is a free pass-through,
/// so newer models still work; this list is only for discovery.
//
// ponytail: hardcoded snapshot, will go stale. To refresh (needs an
// ANTHROPIC_API_KEY, which the subscription `claude` CLI does not expose):
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
/// models (audio, image, embeddings) that codex can't use; those are omitted.
//
// To refresh (needs an OPENAI_API_KEY), keep only the coding models, i.e. the
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

/// Known models for the `grok` harness (xAI Grok Build CLI). Snapshot from
/// `grok models` on 2026-07-09. `--model` is a free pass-through.
//
// To refresh: `grok models` (requires login).
pub const GROK_MODELS: &[&str] = &["grok-4.5", "grok-composer-2.5-fast"];

/// Reasoning-effort levels accepted by the `grok` harness for the default
/// model (`grok-4.5`). Empirically verified 2026-07-09 on grok 0.2.93:
/// `none` is a CLI-parseable value but the API rejects it for grok-4.5;
/// `minimal`/`low`/`medium`/`high`/`xhigh`/`max` all work (`max` aliases
/// `xhigh`). `grok-composer-2.5-fast` ignores effort entirely.
// To update: `grok -p … --effort <level>` against the default model, and
// `~/.grok/docs/user-guide/14-headless-mode.md` for the CLI-parseable set.
pub const GROK_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max"];

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

/// Grok price snapshot: (model, input, cached-input, output) in USD per 1M
/// tokens. grok does not report cost in headless JSON, so dollars are derived.
//
// grok-4.5: docs.x.ai pricing (verified 2026-07-09) — $2 / $0.50 / $6.
// grok-composer-2.5-fast: Cursor Composer 2.5 Fast list rates ($3 / $15);
// xAI does not publish Composer rates (subscription-bundled). Cached input
// is unpublished for Composer — 0.1× input ($0.30), OpenAI-style convention.
// First entry is the assumed default when `--model` is omitted or unknown.
//
// ponytail: Composer cache rate is guessed; refresh when xAI/Cursor publish it.
pub const GROK_PRICES: &[(&str, f64, f64, f64)] = &[
    ("grok-4.5", 2.0, 0.50, 6.0),
    ("grok-composer-2.5-fast", 3.0, 0.30, 15.0),
];
