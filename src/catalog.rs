//! Static harness → model → effort dependency graph, plus price snapshots.
//!
//! Known models are validated against their harness; unknown models remain
//! free pass-through (new provider ids still work). Effort is validated
//! against the selected model when known, otherwise against the harness
//! default model (first entry).

/// Shared effort levels for all known Claude models.
// To update: `claude --help` lists the values on the `--effort <level>` line.
// Verified 2026-10-14 on Claude Code 2.1.300.
pub const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Effort levels for `gpt-6.1-sol` (harness default) and the other non-luna
/// GPT-6 / GPT-5.6 models. From `codex debug models` on 0.162.1 (2026-10-14).
// To update: `codex debug models` → `supported_reasoning_levels`.
pub const CODEX_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];

/// Effort levels for the luna models (`ultra` is not advertised).
const CODEX_LUNA_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Effort levels for `grok-4.7` (harness default), `grok-4.7-build-fast`, and
/// `grok-4.6`. Empirically verified 2026-10-14 on grok 1.0.52: advertised menu
/// is `low`/`medium`/`high`/`xhigh`. `none`, `minimal`, and `max` are rejected.
// To update: `grok -p x --effort bogus` prints the accepted list.
pub const GROK_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh"];

/// Effort levels for `grok-4.5`. Empirically verified 2026-10-14 on grok
/// 1.0.52: advertised menu is `low`/`medium`/`high` (no `xhigh`).
const GROK_45_EFFORTS: &[&str] = &["low", "medium", "high"];

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
// Claude models: Claude Code 2.1.300 (2026-10-14). Default is Opus 5.5. Full
// ids are the `canonicalModel` each alias (opus/sonnet/fable/haiku) resolves
// to in `-p --output-format json`, plus older ids that still resolve to
// themselves (claude-opus-4-1-20250805 now silently resolves to Opus 5.5, so
// it is dropped). Needs ANTHROPIC_API_KEY to refresh from the API:
//   curl -s https://api.anthropic.com/v1/models \
//     -H "x-api-key: $ANTHROPIC_API_KEY" -H "anthropic-version: 2023-06-01" \
//     | python3 -c 'import json,sys; [print(m["id"]) for m in json.load(sys.stdin)["data"]]'
//
// Codex models: live `codex debug models` on 0.162.1 (2026-10-14); first
// entry is the CLI default (gpt-6.1-sol with an empty CODEX_HOME). gpt-5.5,
// gpt-5.4, gpt-5.4-mini, and gpt-5.3-codex-spark left the picker and are
// rejected on ChatGPT accounts.
// Grok models: `grok models` (requires login); 2026-10-14 on grok 1.0.52.
pub const HARNESSES: &[HarnessInfo] = &[
    HarnessInfo {
        name: "claude",
        models: &[
            Model { id: "claude-opus-5-5", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-sonnet-5-5", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-fable-5-1", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-haiku-5-5", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-opus-5", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-sonnet-5", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-fable-5", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-opus-4-8", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-opus-4-7", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-sonnet-4-6", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-opus-4-6", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-opus-4-5-20251101", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-haiku-4-5-20251001", efforts: CLAUDE_EFFORTS },
            Model { id: "claude-sonnet-4-5-20250929", efforts: CLAUDE_EFFORTS },
        ],
    },
    HarnessInfo {
        name: "codex",
        // Live picker order (visibility=list) on 0.162.1; first entry is default.
        models: &[
            Model { id: "gpt-6.1-sol", efforts: CODEX_EFFORTS },
            Model { id: "gpt-6-astra", efforts: CODEX_EFFORTS },
            Model { id: "gpt-6-sol", efforts: CODEX_EFFORTS },
            Model { id: "gpt-6-luna", efforts: CODEX_LUNA_EFFORTS },
            Model { id: "gpt-5.6-sol", efforts: CODEX_EFFORTS },
            Model { id: "gpt-5.6-terra", efforts: CODEX_EFFORTS },
            Model { id: "gpt-5.6-luna", efforts: CODEX_LUNA_EFFORTS },
        ],
    },
    HarnessInfo {
        name: "grok",
        models: &[
            Model { id: "grok-4.7", efforts: GROK_EFFORTS },
            Model { id: "grok-4.7-build-fast", efforts: GROK_EFFORTS },
            Model { id: "grok-4.6", efforts: GROK_EFFORTS },
            Model { id: "grok-4.5", efforts: GROK_45_EFFORTS },
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
    // Unknown model on this harness: fall through to harness default efforts.
    if let Some(id) = model
        && let Some(m) = h.models.iter().find(|m| m.id == id)
    {
        return Ok(m.efforts);
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
    if let Some(id) = model
        && let Some(owner) = harness_for_model(id)
        && owner != harness
    {
        return Err(format!(
            "model '{id}' is for harness '{owner}', not '{harness}'; try --harness {owner}"
        ));
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

/// Codex price snapshot: (model, input, cached-input, cache-write, output)
/// in USD per 1M tokens. codex does not report cost, so dollar output is
/// derived from this.
//
// Verified 2026-10-14 against https://developers.openai.com/api/docs/pricing
// (short-context standard rates). gpt-5.6-sol promotional pricing is listed
// through at least 2026-11-21. Cache writes are listed at 1.25x input for
// GPT-6 / GPT-5.6; older models list none, so they use the input rate.
// gpt-5.3-codex-spark is not listed; rate is inferred from gpt-5.3-codex.
// Legacy ids kept so pass-through burns still price correctly.
// First entry is the assumed default when `--model` is omitted or unknown.
pub const CODEX_PRICES: &[(&str, f64, f64, f64, f64)] = &[
    ("gpt-6.1-sol", 2.0, 0.10, 2.50, 10.0),
    ("gpt-6-astra", 10.0, 1.0, 12.50, 50.0),
    ("gpt-6-sol", 2.0, 0.20, 2.50, 10.0),
    ("gpt-6-luna", 0.10, 0.01, 0.125, 0.50),
    ("gpt-5.6-sol", 4.0, 0.40, 5.0, 20.0),
    ("gpt-5.6-terra", 2.0, 0.20, 2.50, 12.0),
    ("gpt-5.6-luna", 0.20, 0.02, 0.25, 1.20),
    // legacy ids (still pass through if the harness accepts them)
    ("gpt-5.5", 5.0, 0.50, 5.0, 30.0),
    ("gpt-5.4", 2.50, 0.25, 2.50, 15.0),
    ("gpt-5.4-mini", 0.75, 0.075, 0.75, 4.50),
    ("gpt-5.3-codex-spark", 1.75, 0.175, 1.75, 14.0), // inferred from gpt-5.3-codex
    ("gpt-5.3-codex", 1.75, 0.175, 1.75, 14.0),
    ("gpt-5.2-codex", 1.75, 0.175, 1.75, 14.0),
    ("gpt-5.1-codex-max", 1.25, 0.125, 1.25, 10.0),
    ("gpt-5.1-codex-mini", 0.25, 0.025, 0.25, 2.0),
    ("gpt-5.1-codex", 1.25, 0.125, 1.25, 10.0),
    ("gpt-5-codex", 1.25, 0.125, 1.25, 10.0),
];

/// Grok price snapshot: (model, input, cached-input, output) in USD per 1M
/// tokens. Fallback when headless JSON omits `total_cost_usd`.
//
// docs.x.ai pricing (verified 2026-10-14), below 200k prompt tokens (≥200k
// is 2×; not modeled here). grok-4.7 / grok-4.6: $2 / $0.50 / $6.
// grok-4.5: $2 / $0.30 / $6. grok-4.7-build-fast is not on the API price
// list; xAI bills it at 2× grok-4.7.
// First entry is the assumed default when `--model` is omitted or unknown.
pub const GROK_PRICES: &[(&str, f64, f64, f64)] = &[
    ("grok-4.7", 2.0, 0.50, 6.0),
    ("grok-4.7-build-fast", 4.0, 1.0, 12.0),
    ("grok-4.6", 2.0, 0.50, 6.0),
    ("grok-4.5", 2.0, 0.30, 6.0),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_for_model_resolves_owners() {
        assert_eq!(harness_for_model("grok-4.6"), Some("grok"));
        assert_eq!(harness_for_model("grok-4.5"), Some("grok"));
        assert_eq!(harness_for_model("grok-4.7"), Some("grok"));
        assert_eq!(harness_for_model("claude-opus-5-5"), Some("claude"));
        assert_eq!(harness_for_model("claude-opus-5"), Some("claude"));
        assert_eq!(harness_for_model("claude-opus-4-8"), Some("claude"));
        assert_eq!(harness_for_model("gpt-6.1-sol"), Some("codex"));
        assert_eq!(harness_for_model("gpt-5.6-sol"), Some("codex"));
        assert_eq!(harness_for_model("totally-unknown"), None);
    }

    #[test]
    fn wrong_harness_for_known_model_errors() {
        let err = validate_selection("claude", Some("grok-4.6"), None).unwrap_err();
        assert!(err.contains("grok"), "err was: {err}");
        assert!(err.contains("--harness grok"), "err was: {err}");
    }

    #[test]
    fn grok_46_accepts_xhigh_rejects_minimal() {
        assert!(validate_selection("grok", Some("grok-4.6"), Some("xhigh")).is_ok());
        let err = validate_selection("grok", Some("grok-4.6"), Some("minimal")).unwrap_err();
        assert!(err.contains("invalid effort 'minimal'"), "err was: {err}");
        assert!(err.contains("model 'grok-4.6'"), "err was: {err}");
    }

    #[test]
    fn grok_45_accepts_high_rejects_xhigh() {
        assert!(validate_selection("grok", Some("grok-4.5"), Some("high")).is_ok());
        let err = validate_selection("grok", Some("grok-4.5"), Some("xhigh")).unwrap_err();
        assert!(err.contains("invalid effort 'xhigh'"), "err was: {err}");
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
        assert!(validate_selection("codex", None, Some("ultra")).is_ok());
        assert!(validate_selection("codex", None, Some("xhigh")).is_ok());
        assert!(validate_selection("codex", None, Some("minimal")).is_err());
    }

    #[test]
    fn gpt56_luna_rejects_ultra() {
        assert!(validate_selection("codex", Some("gpt-5.6-luna"), Some("max")).is_ok());
        let err = validate_selection("codex", Some("gpt-5.6-luna"), Some("ultra")).unwrap_err();
        assert!(err.contains("invalid effort 'ultra'"), "err was: {err}");
        assert!(err.contains("model 'gpt-5.6-luna'"), "err was: {err}");
    }

    #[test]
    fn gpt6_luna_rejects_ultra() {
        assert!(validate_selection("codex", Some("gpt-6-luna"), Some("max")).is_ok());
        let err = validate_selection("codex", Some("gpt-6-luna"), Some("ultra")).unwrap_err();
        assert!(err.contains("invalid effort 'ultra'"), "err was: {err}");
        assert!(err.contains("model 'gpt-6-luna'"), "err was: {err}");
    }

    #[test]
    fn grok_47_rejects_max() {
        assert!(validate_selection("grok", None, Some("xhigh")).is_ok());
        let err = validate_selection("grok", Some("grok-4.7"), Some("max")).unwrap_err();
        assert!(err.contains("invalid effort 'max'"), "err was: {err}");
        assert!(err.contains("model 'grok-4.7'"), "err was: {err}");
    }
}
