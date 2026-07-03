use std::io::Write;
use std::time::{Duration, Instant};

use brulr::{
    burn, calibrate, Burner, ClaudeBurner, CodexBurner, Rng, CLAUDE_EFFORTS, CLAUDE_MODELS,
    CODEX_EFFORTS, CODEX_MODELS, PROBES,
};
use chrono::{Local, Timelike};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Clone, Copy, ValueEnum)]
enum Harness {
    Claude,
    Codex,
}

#[derive(Parser)]
#[command(name = "brülr", version, about = "A CLI for burning AI tokens on purpose.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Burn tokens with the overhead + random-padding strategy.
    Burn {
        /// Token count (e.g. 100000) or duration (e.g. 90s, 45m, 2h).
        #[arg(default_value = "100000")]
        target: String,
        /// Burn until the next local wall-clock time HH:MM (overrides target).
        #[arg(long)]
        until: Option<String>,
        /// Agent harness CLI to burn against.
        #[arg(long, value_enum, default_value_t = Harness::Claude)]
        harness: Harness,
        /// Model to pass to the harness (see `brulr models`; default is the
        /// harness's own default).
        #[arg(long)]
        model: Option<String>,
        /// Reasoning effort (claude: low/medium/high/xhigh/max; codex:
        /// minimal/low/medium/high). Default: the harness/model default.
        #[arg(long)]
        effort: Option<String>,
    },
    /// List the known models for a harness (snapshot; any model still works).
    Models {
        /// Harness to list models for; omit to list all.
        #[arg(long, value_enum)]
        harness: Option<Harness>,
    },
}

/// Parse "HH:MM" into seconds-of-day.
fn parse_hhmm(s: &str) -> Result<u32, String> {
    let bad = || format!("invalid time: {s} (use HH:MM, 24-hour)");
    let (h, m) = s.split_once(':').ok_or_else(bad)?;
    let h: u32 = h.parse().map_err(|_| bad())?;
    let m: u32 = m.parse().map_err(|_| bad())?;
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok(h * 3600 + m * 60)
}

/// Seconds from `now` to the next occurrence of `target` on a 24h clock.
/// Exact match maps to a full day rather than zero (don't burn nothing).
// ponytail: assumes 86400s/day, so a DST change mid-window shifts the stop by
// an hour — fine for a burn tool.
fn secs_until(now: u32, target: u32) -> u64 {
    const DAY: u32 = 86_400;
    match (target + DAY - now) % DAY {
        0 => DAY as u64,
        n => n as u64,
    }
}

/// Parse a burn target: plain integer = tokens, integer + s/m/h = duration.
fn parse_target(s: &str) -> Result<(u64, Option<Duration>), String> {
    let (num, unit) = s.split_at(s.len() - s.chars().last().map_or(0, |c| c.len_utf8()));
    let secs_per_unit = match unit {
        "s" => Some(1),
        "m" => Some(60),
        "h" => Some(3600),
        _ => None,
    };
    match secs_per_unit {
        Some(mult) => {
            let n: u64 = num
                .parse()
                .map_err(|_| format!("invalid duration: {s} (use e.g. 90s, 45m, 2h)"))?;
            Ok((u64::MAX, Some(Duration::from_secs(n * mult))))
        }
        None => {
            let n: u64 = s
                .parse()
                .map_err(|_| format!("invalid target: {s} (tokens like 100000, or 90s/45m/2h)"))?;
            Ok((n, None))
        }
    }
}

/// Compact duration for display: 90s, 45m, 2h, 1h30m.
fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    let mut out = String::new();
    if h > 0 {
        out += &format!("{h}h");
    }
    if m > 0 {
        out += &format!("{m}m");
    }
    if sec > 0 || out.is_empty() {
        out += &format!("{sec}s");
    }
    out
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Burn { target, until, harness, model, effort } => {
            let parsed = match &until {
                Some(hhmm) => parse_hhmm(hhmm).map(|target_sod| {
                    let now = Local::now();
                    let now_sod = now.hour() * 3600 + now.minute() * 60 + now.second();
                    (u64::MAX, Some(Duration::from_secs(secs_until(now_sod, target_sod))))
                }),
                None => parse_target(&target),
            };
            let (target, duration) = match parsed {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            };
            let mut rng = Rng::from_entropy();
            let harness_name = match harness {
                Harness::Claude => "claude",
                Harness::Codex => "codex",
            };
            let efforts = match harness {
                Harness::Claude => CLAUDE_EFFORTS,
                Harness::Codex => CODEX_EFFORTS,
            };
            if let Some(e) = &effort {
                if !efforts.contains(&e.as_str()) {
                    eprintln!(
                        "error: invalid effort '{e}' for {harness_name}; accepted: {}",
                        efforts.join(", "),
                    );
                    std::process::exit(2);
                }
            }
            let goal = match duration {
                Some(d) => fmt_dur(d),
                None => format!("{target} tokens"),
            };
            eprintln!(
                "burning via {harness_name} · model: {} · effort: {} · {goal}",
                model.as_deref().unwrap_or("default"),
                effort.as_deref().unwrap_or("default"),
            );
            let mut burner: Box<dyn Burner> = match harness {
                Harness::Claude => Box::new(ClaudeBurner { model, effort }),
                Harness::Codex => Box::new(CodexBurner { model, effort }),
            };

            eprint!("\r  calibrating… 0/{PROBES} probes");
            let _ = std::io::stderr().flush();
            let mut on_probe = |r: &brulr::Report| {
                eprint!(
                    "\r  calibrating… {}/{PROBES} probes · {} tokens   ",
                    r.calls,
                    r.processed(),
                );
                let _ = std::io::stderr().flush();
            };
            let (cal, report) = match calibrate(&mut rng, burner.as_mut(), &mut on_probe) {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("\nerror: {e}");
                    std::process::exit(1);
                }
            };
            eprintln!(
                "\r  calibrated: ~{:.0} tokens/call overhead, {:.2} tokens/byte ({} tokens burned calibrating)",
                cal.overhead,
                cal.tokens_per_byte,
                report.processed(),
            );

            // The deadline clock starts after calibration — `burn 45m` means
            // 45 minutes of burning, not 45 minutes minus setup.
            let start = Instant::now();
            let deadline = duration.map(|d| start + d);
            let baseline = report.processed(); // exclude calibration tokens from the rate
            let mut progress = |r: &brulr::Report| {
                let secs = start.elapsed().as_secs_f64();
                let burned = r.processed().saturating_sub(baseline);
                let rate = if secs > 0.1 { burned as f64 / secs } else { 0.0 };
                match duration {
                    Some(d) => {
                        let total = d.as_secs_f64();
                        let pct = (secs / total * 100.0).min(100.0);
                        let left = (total - secs).max(0.0);
                        eprint!(
                            "\r  {pct:3.0}% · {left:.0}s left · {} tokens · {} calls · {rate:.0} tok/s   ",
                            r.processed(),
                            r.calls,
                        );
                    }
                    None => {
                        let pct = (r.processed() as f64 / target as f64 * 100.0).min(100.0);
                        let eta = if rate > 0.0 {
                            (target.saturating_sub(r.processed())) as f64 / rate
                        } else {
                            0.0
                        };
                        eprint!(
                            "\r  {pct:3.0}% · {}/{target} tokens · {} calls · {rate:.0} tok/s · ETA {eta:.0}s   ",
                            r.processed(),
                            r.calls,
                        );
                    }
                }
                let _ = std::io::stderr().flush();
            };
            progress(&report); // paint current progress before the first (slow) burn call
            match burn(target, deadline, &cal, report, &mut rng, burner.as_mut(), &mut progress) {
                Ok(r) => {
                    eprintln!(); // finish the progress line
                    println!("calls:              {}", r.calls);
                    println!("input tokens:       {}", r.input_tokens);
                    println!("cache-write tokens: {}", r.cache_creation_input_tokens);
                    println!("output tokens:      {}", r.output_tokens);
                    println!("cache-read tokens:  {}", r.cache_read_input_tokens);
                    println!("raw tokens:         {}  (face value — leaderboard number)", r.raw_tokens());
                    println!("cost-weighted:      {:.0}  (cache reads at 0.1x — real burn)", r.cost_weighted_tokens());
                    if r.cache_hit_ratio() > 0.1 {
                        eprintln!(
                            "warning: {:.0}% of input served from cache — padding is being cached, burn is not real",
                            r.cache_hit_ratio() * 100.0
                        );
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Cmd::Models { harness } => {
            let list = |name: &str, models: &[&str]| {
                println!("{name}:");
                for m in models {
                    println!("  {m}");
                }
            };
            match harness {
                Some(Harness::Claude) => list("claude", CLAUDE_MODELS),
                Some(Harness::Codex) => list("codex", CODEX_MODELS),
                None => {
                    list("claude", CLAUDE_MODELS);
                    list("codex", CODEX_MODELS);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_target_handles_tokens_and_durations() {
        assert_eq!(parse_target("100000").unwrap(), (100_000, None));
        assert_eq!(
            parse_target("90s").unwrap(),
            (u64::MAX, Some(Duration::from_secs(90)))
        );
        assert_eq!(
            parse_target("45m").unwrap(),
            (u64::MAX, Some(Duration::from_secs(45 * 60)))
        );
        assert_eq!(
            parse_target("2h").unwrap(),
            (u64::MAX, Some(Duration::from_secs(2 * 3600)))
        );
        assert!(parse_target("45x").is_err());
        assert!(parse_target("m").is_err());
        assert!(parse_target("").is_err());
    }

    #[test]
    fn fmt_dur_is_compact() {
        assert_eq!(fmt_dur(Duration::from_secs(90)), "1m30s");
        assert_eq!(fmt_dur(Duration::from_secs(45 * 60)), "45m");
        assert_eq!(fmt_dur(Duration::from_secs(2 * 3600)), "2h");
        assert_eq!(fmt_dur(Duration::from_secs(0)), "0s");
    }

    #[test]
    fn parse_hhmm_valid_and_invalid() {
        assert_eq!(parse_hhmm("00:00").unwrap(), 0);
        assert_eq!(parse_hhmm("07:00").unwrap(), 7 * 3600);
        assert_eq!(parse_hhmm("23:59").unwrap(), 23 * 3600 + 59 * 60);
        assert!(parse_hhmm("24:00").is_err());
        assert!(parse_hhmm("07:60").is_err());
        assert!(parse_hhmm("0700").is_err());
        assert!(parse_hhmm("").is_err());
    }

    #[test]
    fn secs_until_covers_before_after_and_exact() {
        // target later today
        assert_eq!(secs_until(6 * 3600, 7 * 3600), 3600);
        // target already passed → next day
        assert_eq!(secs_until(8 * 3600, 7 * 3600), 23 * 3600);
        // exact match → full day, never zero
        assert_eq!(secs_until(7 * 3600, 7 * 3600), 86_400);
    }
}
