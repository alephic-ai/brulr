use clap::Parser;

#[derive(Parser)]
#[command(name = "brülr", version, about = "A CLI for burning AI tokens on purpose.")]
struct Cli {}

fn main() {
    Cli::parse();
}
