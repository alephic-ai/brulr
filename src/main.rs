mod target;

use std::io::Write;
use std::time::{Duration, Instant};

use brulr::{
    burn, calibrate, validate_selection, Burner, ClaudeBurner, CodexBurner, GrokBurner, Rng,
    CLAUDE_MODELS, CODEX_MODELS, GROK_MODELS, PROBES,
};
use chrono::{Local, Timelike};
use clap::{Parser, Subcommand, ValueEnum};
use target::{fmt_dur, parse_hhmm, parse_target, secs_until, Goal};

#[derive(Clone, Copy, ValueEnum)]
enum Harness {
    Claude,
    Codex,
    Grok,
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
        /// Token count (e.g. 100000), duration (e.g. 90s, 45m, 2h), or dollar
        /// amount (e.g. 5usd, 0.25usd).
        #[arg(default_value = "100000")]
        target: String,
        /// Burn until the next local wall-clock time HH:MM (overrides target).
        #[arg(long)]
        until: Option<String>,
        /// Agent harness CLI to burn against.
        #[arg(long, value_enum, default_value_t = Harness::Claude)]
        harness: Harness,
        /// Model to pass to the harness (see `brulr models`; default is the
        /// harness's own default). Known models must match --harness.
        #[arg(long)]
        model: Option<String>,
        /// Reasoning effort for the selected model (see harness docs). Rejected
        /// when the model does not support effort (e.g. grok-composer-2.5-fast).
        /// Default: the harness/model default.
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

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Burn { target, until, harness, model, effort } => {
            let parsed = match &until {
                Some(hhmm) => parse_hhmm(hhmm).map(|target_sod| {
                    let now = Local::now();
                    let now_sod = now.hour() * 3600 + now.minute() * 60 + now.second();
                    Goal::Duration(Duration::from_secs(secs_until(now_sod, target_sod)))
                }),
                None => parse_target(&target),
            };
            let goal = match parsed {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            };
            let (target, duration, target_usd) = match goal {
                Goal::Tokens(n) => (n, None, None),
                Goal::Duration(d) => (u64::MAX, Some(d), None),
                Goal::Dollars(u) => (u64::MAX, None, Some(u)),
            };
            let mut rng = Rng::from_entropy();
            let harness_name = match harness {
                Harness::Claude => "claude",
                Harness::Codex => "codex",
                Harness::Grok => "grok",
            };
            if let Err(e) =
                validate_selection(harness_name, model.as_deref(), effort.as_deref())
            {
                eprintln!("error: {e}");
                std::process::exit(2);
            }
            let goal_label = match (duration, target_usd) {
                (Some(d), _) => fmt_dur(d),
                (_, Some(u)) => format!("${u:.2}"),
                _ => format!("{target} tokens"),
            };
            eprintln!(
                "burning via {harness_name} · model: {} · effort: {} · {goal_label}",
                model.as_deref().unwrap_or("default"),
                effort.as_deref().unwrap_or("default"),
            );
            let mut burner: Box<dyn Burner> = match harness {
                Harness::Claude => Box::new(ClaudeBurner { model, effort }),
                Harness::Codex => Box::new(CodexBurner { model, effort }),
                Harness::Grok => Box::new(GrokBurner { model, effort }),
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

            // The deadline clock starts after calibration, so `burn 45m` means
            // 45 minutes of burning, not 45 minutes minus setup.
            let start = Instant::now();
            let deadline = duration.map(|d| start + d);
            let baseline = report.processed(); // exclude calibration tokens from the rate
            let mut progress = |r: &brulr::Report| {
                let secs = start.elapsed().as_secs_f64();
                let burned = r.processed().saturating_sub(baseline);
                let rate = if secs > 0.1 { burned as f64 / secs } else { 0.0 };
                match (duration, target_usd) {
                    (Some(d), _) => {
                        let total = d.as_secs_f64();
                        let pct = (secs / total * 100.0).min(100.0);
                        let left = (total - secs).max(0.0);
                        eprint!(
                            "\r  {pct:3.0}% · {left:.0}s left · {} tokens · {} calls · {rate:.0} tok/s · ${:.2}   ",
                            r.processed(),
                            r.calls,
                            r.cost_usd,
                        );
                    }
                    (_, Some(u)) => {
                        let pct = (r.cost_usd / u * 100.0).min(100.0);
                        eprint!(
                            "\r  {pct:3.0}% · ${:.2}/${u:.2} · {} tokens · {} calls · {rate:.0} tok/s   ",
                            r.cost_usd,
                            r.processed(),
                            r.calls,
                        );
                    }
                    _ => {
                        let pct = (r.processed() as f64 / target as f64 * 100.0).min(100.0);
                        let eta = if rate > 0.0 {
                            (target.saturating_sub(r.processed())) as f64 / rate
                        } else {
                            0.0
                        };
                        eprint!(
                            "\r  {pct:3.0}% · {}/{target} tokens · {} calls · {rate:.0} tok/s · ETA {eta:.0}s · ${:.2}   ",
                            r.processed(),
                            r.calls,
                            r.cost_usd,
                        );
                    }
                }
                let _ = std::io::stderr().flush();
            };
            progress(&report); // paint current progress before the first (slow) burn call
            match burn(target, deadline, target_usd, &cal, report, &mut rng, burner.as_mut(), &mut progress) {
                Ok(r) => {
                    eprintln!(); // finish the progress line
                    println!("calls:              {}", r.calls);
                    println!("input tokens:       {}", r.input_tokens);
                    println!("cache-write tokens: {}", r.cache_creation_input_tokens);
                    println!("output tokens:      {}", r.output_tokens);
                    println!("cache-read tokens:  {}", r.cache_read_input_tokens);
                    println!("raw tokens:         {}  (face value, leaderboard number)", r.raw_tokens());
                    println!("cost-weighted:      {:.0}  (cache reads at 0.1x, real burn)", r.cost_weighted_tokens());
                    println!("cost:               ${:.4}  (API-equivalent)", r.cost_usd);
                    // ponytail: 0.25 clears the ~10% floor from codex's cached
                    // fixed preamble; real padding absorption blows way past it.
                    if r.cache_hit_ratio() > 0.25 {
                        eprintln!(
                            "warning: {:.0}% of input served from cache; padding is being cached, burn is not real",
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
                Some(Harness::Grok) => list("grok", GROK_MODELS),
                None => {
                    list("claude", CLAUDE_MODELS);
                    list("codex", CODEX_MODELS);
                    list("grok", GROK_MODELS);
                }
            }
        }
    }
}
