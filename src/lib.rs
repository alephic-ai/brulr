//! brülr: burn AI tokens on purpose.
//!
//! The overhead strategy: send many calls, each padded with uncacheable
//! random bytes. Input tokens ingest far faster than output generates, so
//! padding maximizes burn per call at minimal latency. A fresh random pad
//! per call defeats prefix caching (which dies on the first divergent token).
//!
//! The loop is backend-agnostic via [`Burner`]; [`ClaudeBurner`] is the real
//! backend and tests use a fake one.

mod backend;
mod burn;
mod calibrate;
mod catalog;
mod rng;
mod usage;

pub use backend::{
    codex_cost, grok_cost, parse_codex_usage, parse_grok_session_id, parse_grok_usage_from_log,
    parse_usage, Burner, ClaudeBurner, CodexBurner, GrokBurner,
};
pub use burn::burn;
pub use calibrate::{calibrate, Calibration, MAX_PAD_BYTES, PROBES};
pub use catalog::{
    efforts_for, harness_for_model, harness_info, models_for_harness, validate_selection,
    CLAUDE_EFFORTS, CODEX_EFFORTS, CODEX_PRICES, GROK_EFFORTS,
    GROK_PRICES, HARNESSES, HarnessInfo, Model,
};
pub use rng::{build_prompt, Rng};
pub use usage::{Report, Usage, CACHE_READ_WEIGHT};
