//! Static harness → model → effort dependency graph, plus price snapshots.
//!
//! Known models are validated against their harness; unknown models remain
//! free pass-through (new provider ids still work). Effort is validated
//! against the selected model when known, otherwise against the harness
//! default model (first entry).

/// Shared effort levels for all known Claude models.
// To update: `claude --help` lists the values on the `--effort <level>` line.
pub const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Shared effort levels for all known Codex models.
// To update: see codex's `model_reasoning_effort` config documentation.
pub const CODEX_EFFORTS: &[&str] = &["minimal", "low", "medium", "high"];

/// Effort levels for `grok-4.5` (harness default). Empirically verified
/// 2026-07-09 on grok 0.2.93: `none` is rejected by the API; the rest work
/// (`max` aliases `xhigh`).
// To update: `grok -p … --effort <level>` against grok-4.5.
pub const GROK_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max"];

/// Empty effort set: model does not support `--effort`.
const NO_EFFORTS: &[&str] = &[];

/// One known model and the reasoning-effort levels it accepts.
/// `efforts` empty means `--effort` is not allowed on this model.
#[derive(Debug, Clone, Copy)]
pub struct Model {
    pub id: &'static str,
    pub efforts: &'static [&'static str],
}

/// One harness and its known models (first model is the default for
/// effort validation when `--model` is omitted or unknown).
#[derive(Debug, Clone, Copy)]
pub struct HarnessInfo {
    pub name: &'static str,
    pub models: &'static [Model],
}

/// Source of truth: harness → models → efforts.
///
/// Model tables are snapshots for discovery and validation; newer ids not
/// listed here still work as pass-through on their harness.
//
// Claude models: 2026-07-03 from the Anthropic model API (needs ANTHROPIC_API_KEY):
//   curl -s https://api.anthropic.com/v1/models \
//     -H "x-api-key: $ANTHROPIC_API_KEY" -H "anthropic-version: 2023-06-01" \
//     | python3 -c 'import json,sys; [print(m["id"]) for m in json.load(sys.stdin)["data"]]'
//
// Codex models: coding ids only from OpenAI model API (needs OPENAI_API_KEY).
// Grok models: `grok models` (requires login); 2026-07-09.
pub const HARNESSES: &[HarnessInfo] = &[
    HarnessInfo {
        name: "claude",
        models: &[
            Model { id: "claude-sonnet-5", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-fable-5", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-opus-4-8", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-opus-4-7", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-sonnet-4-6", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-opus-4-6", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-opus-4-5-20251101", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-haiku-4-5-20251001", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-sonnet-4-5-20250929", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-opus-4-1-20250805", efforts: CLAUDE_EFFORTS },
        ],
    },
    HarnessInfo {
        name: "codex",
        models: &[
            Model { id: "gpt-5.3-codex", efforts: CODEX_EFFORTS },
            Model { id: "gpt-5.2-codex", efforts: CODEX_EFFORTS },
            Model { id: "gpt-5.1-codex-max", efforts: CODEX_EFFORTS },
            Model { id: "gpt-5.1-codex-mini", efforts: CODEX_EFFORTS },
            Model { id: "gpt-5.1-codex", efforts: CODEX_EFFORTS },
            Model { id: "gpt-5-codex", efforts: CODEX_EFFORTS },
        ],
    },
    HarnessInfo {
        name: "grok",
        models: &[
            Model { id: "grok-4.5", efforts: GROK_EFFORTS },
            // Composer ignores effort; empty set rejects --effort explicitly.
            Model { id: "grok-composer-2.5-fast", efforts: NO_EFFORTS },
        ],
    },
];

/// Look up a harness entry by name (`claude` / `codex` / `grok`).
pub fn harness_info(name: &str) -> Option<&'static HarnessInfo> {
    HARNESSES.iter().find(|h| h.name == name)
}

/// Harness name that owns this known model, if any.
pub fn harness_for_model(model: &str) -> Option<&'static str> {
    for h in HARNESSES {
        if h.models.iter().any(|m| m.id == model) {
            return Some(h.name);
        }
    }
    None
}

/// Known models for a harness, if the harness name is recognized.
pub fn models_for_harness(harness: &str) -> Option<&'static [Model]> {
    harness_info(harness).map(|h| h.models)
}

/// Allowed efforts for this harness + optional model.
///
/// - known model on this harness → that model's list (may be empty)
/// - omitted or unknown model → first (default) model's list for the harness
/// - unknown harness → error
pub fn efforts_for(harness: &str, model: Option<&str>) -> Result<&'static [&'static str], String> {
    let h = harness_info(harness)
        .ok_or_else(|| format!("unknown harness '{harness}'"))?;
    if let Some(id) = model {
        if let Some(m) = h.models.iter().find(|m| m.id == id) {
            return Ok(m.efforts);
        }
        // Unknown model on this harness: use harness default efforts.
    }
    h.models
        .first()
        .map(|m| m.efforts)
        .ok_or_else(|| format!("harness '{harness}' has no known models"))
}

/// Validate harness / model / effort against the dependency graph.
///
/// Known models must match the harness. Unknown models are allowed
/// (pass-through). Effort is checked against the resolved model (or harness
/// default when the model is omitted/unknown).
pub fn validate_selection(
    harness: &str,
    model: Option<&str>,
    effort: Option<&str>,
) -> Result<(), String> {
    if harness_info(harness).is_none() {
        return Err(format!("unknown harness '{harness}'"));
    }
    if let Some(id) = model {
        if let Some(owner) = harness_for_model(id) {
            if owner != harness {
                return Err(format!(
                    "model '{id}' is for harness '{owner}', not '{harness}'; try --harness {owner}"
                ));
            }
        }
    }
    let efforts = efforts_for(harness, model)?;
    if let Some(e) = effort {
        if efforts.is_empty() {
            let label = model.unwrap_or("default");
            return Err(format!("model '{label}' does not support --effort"));
        }
        if !efforts.contains(&e) {
            let scope = match model {
                Some(id) if harness_for_model(id) == Some(harness) => {
                    format!("model '{id}'")
                }
                Some(_) | None => format!("harness '{harness}'"),
            };
            return Err(format!(
                "invalid effort '{e}' for {scope}; accepted: {}",
                efforts.join(", "),
            ));
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_for_model_resolves_owners() {
        assert_eq!(harness_for_model("grok-4.5"), Some("grok"));
        assert_eq!(harness_for_model("claude-opus-4-8"), Some("claude"));
        assert_eq!(harness_for_model("gpt-5.3-codex"), Some("codex"));
        assert_eq!(harness_for_model("totally-unknown"), None);
    }

    #[test]
    fn wrong_harness_for_known_model_errors() {
        let err = validate_selection("claude", Some("grok-4.5"), None).unwrap_err();
        assert!(err.contains("grok"), "err was: {err}");
        assert!(err.contains("--harness grok"), "err was: {err}");
    }

    #[test]
    fn composer_rejects_effort() {
        let err =
            validate_selection("grok", Some("grok-composer-2.5-fast"), Some("high")).unwrap_err();
        assert!(err.contains("does not support --effort"), "err was: {err}");
    }

    #[test]
    fn grok_45_accepts_minimal_rejects_none() {
        assert!(validate_selection("grok", Some("grok-4.5"), Some("minimal")).is_ok());
        let err = validate_selection("grok", Some("grok-4.5"), Some("none")).unwrap_err();
        assert!(err.contains("invalid effort 'none'"), "err was: {err}");
        assert!(err.contains("model 'grok-4.5'"), "err was: {err}");
    }

    #[test]
    fn unknown_model_uses_harness_default_efforts() {
        assert!(validate_selection("claude", Some("future-claude-xyz"), Some("low")).is_ok());
        let err =
            validate_selection("claude", Some("future-claude-xyz"), Some("minimal")).unwrap_err();
        assert!(err.contains("invalid effort 'minimal'"), "err was: {err}");
        assert!(err.contains("harness 'claude'"), "err was: {err}");
    }

    #[test]
    fn omitted_model_uses_default_efforts() {
        assert!(validate_selection("codex", None, Some("minimal")).is_ok());
        assert!(validate_selection("codex", None, Some("xhigh")).is_err());
    }
}
