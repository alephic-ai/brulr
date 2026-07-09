//! The `Burner` trait and its `claude` / `codex` / `grok` implementations,
//! plus the usage/cost parsing each one needs.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
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
/// Token usage is not in stdout JSON; it is recovered from the structured log.
pub struct GrokBurner {
    pub model: Option<String>,
    pub effort: Option<String>,
}

impl Burner for GrokBurner {
    fn run(&mut self, prompt: &str) -> Result<Usage, String> {
        let mut cmd = Command::new("grok");
        // ponytail: flags are version-sensitive (grok 0.2.x). Empty tools +
        // max-turns 1 keeps burns single-turn and off the filesystem.
        cmd.args([
            "-p",
            prompt,
            "--output-format",
            "json",
            "--tools",
            "",
            "--max-turns",
            "1",
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
        let session_id = parse_grok_session_id(&stdout)?;
        let mut usage = parse_grok_usage_from_log(&grok_unified_log_path()?, &session_id)?;
        usage.cost_usd = grok_cost(self.model.as_deref(), &usage);
        Ok(usage)
    }
}

/// Path to grok's structured log: `$GROK_HOME/logs/unified.jsonl`, else
/// `$HOME/.grok/logs/unified.jsonl`.
pub fn grok_unified_log_path() -> Result<PathBuf, String> {
    let home = std::env::var_os("GROK_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".grok")))
        .ok_or_else(|| "can't locate grok home (set GROK_HOME or HOME)".to_string())?;
    Ok(home.join("logs").join("unified.jsonl"))
}

/// Extract `sessionId` from grok headless `--output-format json` stdout.
/// Error objects (`{"type":"error",...}`) become a readable failure.
pub fn parse_grok_session_id(stdout: &[u8]) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_slice(stdout).map_err(|e| format!("parse grok json: {e}"))?;
    if v.get("type").and_then(|t| t.as_str()) == Some("error") {
        let msg = v["message"].as_str().unwrap_or("unknown error");
        return Err(format!("grok error: {msg}"));
    }
    v["sessionId"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "no sessionId in grok json output".into())
}

/// Sum token usage from grok's `shell.turn.inference_done` log lines for one
/// `session_id`. Grok reports `prompt_tokens` inclusive of
/// `cached_prompt_tokens`; reasoning tokens count as output.
///
/// The log is shared and append-only, so the session just burned sits at the
/// tail: read the file backwards in chunks and stop once we walk past its
/// lines, instead of re-scanning a file that grows every burn-loop iteration.
//
// ponytail: scrapes `$GROK_HOME/logs/unified.jsonl` because headless JSON has
// no usage fields (grok 0.2.x). Message name and field layout are
// version-sensitive — fail loudly if nothing matches rather than under-count.
pub fn parse_grok_usage_from_log(log_path: &Path, session_id: &str) -> Result<Usage, String> {
    let mut file = File::open(log_path).map_err(|e| {
        format!(
            "can't read grok log {}: {e} (token usage lives there; is grok writing logs?)",
            log_path.display()
        )
    })?;
    let read_err = |e: std::io::Error| format!("read grok log: {e}");
    let mut prompt = 0u64;
    let mut cached = 0u64;
    let mut completion = 0u64;
    let mut reasoning = 0u64;
    let mut seen = false;
    const CHUNK: u64 = 64 * 1024;
    let mut pos = file.seek(SeekFrom::End(0)).map_err(read_err)?;
    let mut carry: Vec<u8> = Vec::new(); // partial first line of the window below
    'chunks: while pos > 0 {
        let take = pos.min(CHUNK) as usize;
        pos -= take as u64;
        file.seek(SeekFrom::Start(pos)).map_err(read_err)?;
        let mut window = vec![0u8; take];
        file.read_exact(&mut window).map_err(read_err)?;
        window.extend_from_slice(&carry);
        let start = if pos == 0 {
            0
        } else if let Some(nl) = window.iter().position(|&b| b == b'\n') {
            nl + 1 // window[..nl] is the tail of a line starting in an earlier chunk
        } else {
            carry = window; // line longer than the chunk: keep growing it
            continue;
        };
        for raw in window[start..].split(|&b| b == b'\n').rev() {
            let line = String::from_utf8_lossy(raw);
            if !line.contains(session_id) {
                // ponytail: a line carrying another session's sid means we've
                // walked past ours (sessions don't interleave while brulr runs
                // one grok at a time); drop this early-stop if that changes.
                if seen && line.contains("\"sid\":\"") {
                    break 'chunks;
                }
                continue;
            }
            if !line.contains("inference_done") {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            if v["sid"].as_str() != Some(session_id) {
                continue;
            }
            if v["msg"].as_str() != Some("shell.turn.inference_done") {
                continue;
            }
            seen = true;
            let ctx = &v["ctx"];
            prompt += ctx["prompt_tokens"].as_u64().unwrap_or(0);
            cached += ctx["cached_prompt_tokens"].as_u64().unwrap_or(0);
            completion += ctx["completion_tokens"].as_u64().unwrap_or(0);
            reasoning += ctx["reasoning_tokens"].as_u64().unwrap_or(0);
        }
        carry = window[..start.saturating_sub(1)].to_vec();
    }
    if !seen {
        return Err(format!(
            "no shell.turn.inference_done for session {session_id} in {}",
            log_path.display()
        ));
    }
    Ok(Usage {
        input_tokens: prompt.saturating_sub(cached),
        cache_creation_input_tokens: 0,
        output_tokens: completion + reasoning,
        cache_read_input_tokens: cached,
        cost_usd: 0.0, // filled in by GrokBurner from GROK_PRICES
    })
}

/// USD cost of `usage` for a grok `model` (None/unknown falls back to the
/// first `GROK_PRICES` entry).
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
    use std::io::Write;

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
    fn parse_grok_session_id_reads_success() {
        let json = br#"{"text":"ok","stopReason":"EndTurn","sessionId":"019f4472-1d0a-7ae0-a026-222cdaa9fbab","requestId":"abc"}"#;
        assert_eq!(
            parse_grok_session_id(json).unwrap(),
            "019f4472-1d0a-7ae0-a026-222cdaa9fbab"
        );
    }

    #[test]
    fn parse_grok_session_id_errs_on_error_object() {
        let json = br#"{"type":"error","message":"Couldn't create session: boom"}"#;
        let err = parse_grok_session_id(json).unwrap_err();
        assert!(err.contains("Couldn't create session"), "err was: {err}");
    }

    #[test]
    fn parse_grok_session_id_errs_without_session() {
        assert!(parse_grok_session_id(br#"{"text":"ok"}"#).is_err());
    }

    #[test]
    fn parse_grok_usage_from_log_maps_fields() {
        // Real grok 0.2.x shape; prompt_tokens is inclusive of cached.
        let sid = "019f4472-1d0a-7ae0-a026-222cdaa9fbab";
        let line = format!(
            r#"{{"ts":"2026-07-09T01:16:08.060Z","src":"shell","pid":1,"lvl":"info","sid":"{sid}","msg":"shell.turn.inference_done","ctx":{{"loop_index":1,"prompt_tokens":17729,"cached_prompt_tokens":11136,"completion_tokens":29,"reasoning_tokens":24}}}}"#
        );
        let dir = std::env::temp_dir().join(format!("brulr-grok-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("unified.jsonl");
        let mut f = File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"msg":"noise","sid":"{sid}"}}
{line}
{{"msg":"shell.turn.inference_done","sid":"other-session","ctx":{{"prompt_tokens":999}}}}"#
        )
        .unwrap();
        let u = parse_grok_usage_from_log(&path, sid).unwrap();
        assert_eq!(u.input_tokens, 17729 - 11136);
        assert_eq!(u.cache_read_input_tokens, 11136);
        assert_eq!(u.output_tokens, 29 + 24);
        assert_eq!(u.cache_creation_input_tokens, 0);
        assert_eq!(u.processed(), 6593 + 53);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_grok_usage_from_log_sums_loops() {
        let sid = "sess-multi";
        let dir = std::env::temp_dir().join(format!("brulr-grok-multi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("unified.jsonl");
        let mut f = File::create(&path).unwrap();
        for (p, c, o, r) in [(1000u64, 100, 10, 5), (2000, 500, 20, 15)] {
            writeln!(
                f,
                r#"{{"sid":"{sid}","msg":"shell.turn.inference_done","ctx":{{"prompt_tokens":{p},"cached_prompt_tokens":{c},"completion_tokens":{o},"reasoning_tokens":{r}}}}}"#
            )
            .unwrap();
        }
        let u = parse_grok_usage_from_log(&path, sid).unwrap();
        assert_eq!(u.input_tokens, (1000 - 100) + (2000 - 500));
        assert_eq!(u.cache_read_input_tokens, 100 + 500);
        assert_eq!(u.output_tokens, 10 + 5 + 20 + 15);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_grok_usage_from_log_reads_only_the_tail() {
        // Hundreds of KB of older-session lines (spanning several reverse-read
        // chunks, so lines straddle chunk boundaries), then our session last.
        let sid = "sess-tail";
        let dir = std::env::temp_dir().join(format!("brulr-grok-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("unified.jsonl");
        let mut f = File::create(&path).unwrap();
        for i in 0..3000 {
            writeln!(
                f,
                r#"{{"sid":"old-{i}","msg":"shell.turn.inference_done","ctx":{{"prompt_tokens":999,"completion_tokens":999,"padding":"{}"}}}}"#,
                "x".repeat(60)
            )
            .unwrap();
        }
        writeln!(
            f,
            r#"{{"sid":"{sid}","msg":"shell.turn.inference_done","ctx":{{"prompt_tokens":1000,"cached_prompt_tokens":100,"completion_tokens":10,"reasoning_tokens":5}}}}"#
        )
        .unwrap();
        let u = parse_grok_usage_from_log(&path, sid).unwrap();
        assert_eq!(u.input_tokens, 1000 - 100); // old sessions not counted
        assert_eq!(u.cache_read_input_tokens, 100);
        assert_eq!(u.output_tokens, 10 + 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_grok_usage_from_log_errs_without_event() {
        let dir = std::env::temp_dir().join(format!("brulr-grok-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("unified.jsonl");
        std::fs::write(&path, b"{\"msg\":\"other\"}\n").unwrap();
        assert!(parse_grok_usage_from_log(&path, "missing").is_err());
        let _ = std::fs::remove_dir_all(&dir);
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
        // Named composer row, not the default.
        let (_, c_inp, c_cached, c_out) = GROK_PRICES[1];
        let c_expected = c_inp + c_cached + c_out;
        assert!((grok_cost(Some(GROK_PRICES[1].0), &usage) - c_expected).abs() < 1e-9);
    }
}
