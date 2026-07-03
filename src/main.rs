use std::io::Write;
use std::time::Instant;

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
        /// Target number of fresh tokens to process.
        #[arg(default_value_t = 100_000)]
        target: u64,
        /// Model to pass to claude (e.g. haiku, opus).
        #[arg(long)]
        model: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Burn { target, model } => {
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

            let start = Instant::now();
            let baseline = report.processed(); // exclude calibration tokens from the rate
            let mut progress = |r: &brulr::Report| {
                let secs = start.elapsed().as_secs_f64();
                let burned = r.processed().saturating_sub(baseline);
                let rate = if secs > 0.1 { burned as f64 / secs } else { 0.0 };
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
                let _ = std::io::stderr().flush();
            };
            progress(&report); // paint current progress before the first (slow) burn call
            match burn(target, &cal, report, &mut rng, &mut burner, &mut progress) {
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
