//! The `Burner` trait and its `claude` / `codex` / `grok` implementations,
//! plus the usage/cost parsing each one needs.

use std::process::Command;

use crate::catalog::{CODEX_PRICES, GROK_PRICES};
use crate::usage::Usage;

/// A backend that executes one prompt and reports its token usage.
pub trait Burner {
    fn run(&mut self, prompt: &str) -> Result<Usage, String>;
}

/// Spawn a harness command, turning a missing binary into a readable message,
/// and return its stdout on success.
fn run_harness(mut cmd: Command, name: &str) -> Result<Vec<u8>, String> {
    let out = cmd.output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!(
            "can't find the `{name}` harness. Install the {name} CLI and make \
             sure it's on your PATH, or pick another with --harness."
        ),
        _ => format!("failed to run {name}: {e}"),
    })?;
    if !out.status.success() {
        return Err(format!(
            "{name} exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

/// The real backend: shells out to the `claude` CLI in one-shot mode.
/// Flags verified on Claude Code 2.1.251: `--tools ""` still disables tools,
/// `--no-session-persistence` still exists, JSON usage still has the same
/// token/cost fields (`thinking_tokens` is a subset of `output_tokens`).
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
        parse_usage(&run_harness(cmd, "claude")?)
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
        let mut usage = parse_codex_usage(&run_harness(cmd, "codex")?)?;
        usage.cost_usd = codex_cost(self.model.as_deref(), &usage);
        Ok(usage)
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
        // `total_cost_usd` sits at the top level of the result, not under usage.
        cost_usd: v["total_cost_usd"].as_f64().unwrap_or(0.0),
    })
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

/// USD cost of `usage` for a codex `model` (None/unknown falls back to the
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

/// The `grok` backend: shells out to the xAI Grok Build CLI in headless mode.
/// Token usage and cost (when stamped) come from `--output-format json`.
pub struct GrokBurner {
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl Burner for GrokBurner {
    fn run(&mut self, prompt: &str) -> Result<Usage, String> {
        let mut cmd = Command::new("grok");
        // ponytail: flags are version-sensitive (grok 1.0.x). `--no-memory`
        // is a hidden legacy flag that still works. `--tools ""` is accepted
        // but does not strip the stock toolset. max-turns must be >1: if the
        // model calls a tool, the final answer needs another agentic turn —
        // with max-turns 1 grok exits "max turns reached". Prefer single-turn
        // burns (tool re-reads pad from cache); 5 is headroom when the model
        // ignores the no-tools ask.
        cmd.args([
            "-p",
            prompt,
            "--output-format",
            "json",
            "--max-turns",
            "5",
            "--no-subagents",
            "--no-memory",
            "--disable-web-search",
        ]);
        if let Some(m) = &self.model {
            cmd.args(["-m", m]);
        }
        if let Some(e) = &self.effort {
            cmd.args(["--effort", e]);
        }
        let stdout = run_harness(cmd, "grok")?;
        let mut usage = parse_grok_usage(&stdout)?;
        if usage.cost_usd == 0.0 {
            usage.cost_usd = grok_cost(self.model.as_deref(), &usage);
        }
        Ok(usage)
    }
}

/// Extract token usage from grok headless `--output-format json` stdout.
/// Error objects (`{"type":"error",...}`) become a readable failure.
///
/// `usage.input_tokens` is uncached only; `output_tokens` already includes
/// reasoning (`reasoning_tokens` is a subset, not added again). `cost_usd` is
/// `total_cost_usd` when the server stamped it, else 0 (caller fills from
/// `GROK_PRICES`).
pub fn parse_grok_usage(stdout: &[u8]) -> Result<Usage, String> {
    let v: serde_json::Value =
        serde_json::from_slice(stdout).map_err(|e| format!("parse grok json: {e}"))?;
    if v.get("type").and_then(|t| t.as_str()) == Some("error") {
        let msg = v["message"].as_str().unwrap_or("unknown error");
        return Err(format!("grok error: {msg}"));
    }
    let Some(u) = v.get("usage") else {
        return Err("no usage in grok json output".into());
    };
    Ok(Usage {
        input_tokens: u["input_tokens"].as_u64().unwrap_or(0),
        cache_creation_input_tokens: u["cache_creation_input_tokens"].as_u64().unwrap_or(0),
        output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
        cache_read_input_tokens: u["cache_read_input_tokens"].as_u64().unwrap_or(0),
        cost_usd: v["total_cost_usd"].as_f64().unwrap_or(0.0),
    })
}

/// USD cost of `usage` for a grok `model` (None/unknown falls back to the
/// first `GROK_PRICES` entry). Used when headless JSON omits `total_cost_usd`.
pub fn grok_cost(model: Option<&str>, usage: &Usage) -> f64 {
    let (_, inp, cached, out) = model
        .and_then(|m| GROK_PRICES.iter().find(|(name, ..)| *name == m).copied())
        .unwrap_or(GROK_PRICES[0]);
    (usage.input_tokens as f64 * inp
        + usage.cache_read_input_tokens as f64 * cached
        + usage.output_tokens as f64 * out)
        / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parse_usage_reads_claude_code_2_1_shape() {
        // Real Claude Code 2.1.251 result; thinking_tokens is a subset of output.
        let json = br#"{"type":"result","total_cost_usd":0.066115,"usage":{"input_tokens":2,"cache_creation_input_tokens":6088,"cache_read_input_tokens":0,"output_tokens":209,"output_tokens_details":{"thinking_tokens":68}},"modelUsage":{"claude-opus-5[1m]":{"canonicalModel":"claude-opus-5"}}}"#;
        let u = parse_usage(json).unwrap();
        assert_eq!(u.input_tokens, 2);
        assert_eq!(u.cache_creation_input_tokens, 6088);
        assert_eq!(u.output_tokens, 209); // thinking is a subset, not added
        assert_eq!(u.cache_read_input_tokens, 0);
        assert!((u.cost_usd - 0.066115).abs() < 1e-9);
        assert_eq!(u.processed(), 6299);
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
        let expected = inp + cached + out; // 1M of each over 1M
        assert!((codex_cost(Some(CODEX_PRICES[0].0), &usage) - expected).abs() < 1e-9);
        // Unknown/None model falls back to the first entry.
        assert!((codex_cost(None, &usage) - expected).abs() < 1e-9);
        assert!((codex_cost(Some("nope"), &usage) - expected).abs() < 1e-9);
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
    fn parse_grok_usage_maps_fields() {
        // Real grok 1.0.13 shape; input_tokens is uncached, output includes reasoning.
        let json = br#"{
            "text":"ok",
            "stopReason":"end_turn",
            "sessionId":"01a04f07-ee01-7a42-b48a-7f834d3295b7",
            "usage":{
                "input_tokens":19172,
                "cache_read_input_tokens":256,
                "cache_creation_input_tokens":0,
                "output_tokens":23,
                "reasoning_tokens":18,
                "total_tokens":19451
            },
            "total_cost_usd":0.0065637
        }"#;
        let u = parse_grok_usage(json).unwrap();
        assert_eq!(u.input_tokens, 19172);
        assert_eq!(u.cache_read_input_tokens, 256);
        assert_eq!(u.cache_creation_input_tokens, 0);
        assert_eq!(u.output_tokens, 23); // reasoning is a subset, not added
        assert!((u.cost_usd - 0.0065637).abs() < 1e-9);
        assert_eq!(u.processed(), 19172 + 23);
    }

    #[test]
    fn parse_grok_usage_errs_on_error_object() {
        let json = br#"{"type":"error","message":"Couldn't create session: boom"}"#;
        let err = parse_grok_usage(json).unwrap_err();
        assert!(err.contains("Couldn't create session"), "err was: {err}");
    }

    #[test]
    fn parse_grok_usage_errs_without_usage() {
        assert!(parse_grok_usage(br#"{"text":"ok","sessionId":"abc"}"#).is_err());
    }

    #[test]
    fn grok_cost_uses_price_table() {
        let usage = Usage {
            input_tokens: 1_000_000,
            cache_read_input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            ..Default::default()
        };
        let (_, inp, cached, out) = GROK_PRICES[0];
        let expected = inp + cached + out;
        assert!((grok_cost(Some(GROK_PRICES[0].0), &usage) - expected).abs() < 1e-9);
        assert!((grok_cost(None, &usage) - expected).abs() < 1e-9);
        assert!((grok_cost(Some("nope"), &usage) - expected).abs() < 1e-9);
        // Named grok-4.5 row, not the default.
        let (_, c_inp, c_cached, c_out) = GROK_PRICES[1];
        let c_expected = c_inp + c_cached + c_out;
        assert!((grok_cost(Some(GROK_PRICES[1].0), &usage) - c_expected).abs() < 1e-9);
    }
}
