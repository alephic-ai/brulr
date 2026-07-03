use std::io::Write;
use std::time::{Duration, Instant};

use brulr::{burn, calibrate, ClaudeBurner, Rng, PROBES};
use clap::{Parser, Subcommand};

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
        /// Model to pass to claude (e.g. haiku, opus).
        #[arg(long)]
        model: Option<String>,
    },
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

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Burn { target, model } => {
            let (target, duration) = match parse_target(&target) {
                Ok(x) => x,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            };
            let mut rng = Rng::from_entropy();
            let mut burner = ClaudeBurner { model };

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
            let (cal, report) = match calibrate(&mut rng, &mut burner, &mut on_probe) {
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
            match burn(target, deadline, &cal, report, &mut rng, &mut burner, &mut progress) {
                Ok(r) => {
                    eprintln!(); // finish the progress line
                    println!("calls:             {}", r.calls);
                    println!("input tokens:      {}", r.input_tokens);
                    println!("output tokens:     {}", r.output_tokens);
                    println!("cache-read tokens: {}", r.cache_read_input_tokens);
                    println!("processed:         {}", r.processed());
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
}
