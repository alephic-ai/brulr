# brülr
A CLI for burning AI tokens on purpose.

brülr drives an agent harness (`claude` or `codex`) in a loop, padding each call
with uncacheable random bytes to burn tokens toward a target — a count, a
duration, or a wall-clock deadline.

## Install

```sh
cargo build --release
```

The binary lands in `target/release/brulr`. It needs whichever harness you burn
against (`claude` and/or `codex`) installed and authenticated.

## Usage

```sh
brulr burn                 # burn 100000 tokens (default), via claude
brulr burn 500000          # burn a token count
brulr burn 45m             # burn for a duration (90s, 45m, 2h)
brulr burn --until 07:00   # burn until the next local 07:00

brulr burn --harness codex # burn via codex instead of claude
brulr burn --model claude-opus-4-8 --effort high
brulr models               # list known models per harness
```

Run `brulr burn --help` for all flags.

### Options

- `--harness <claude|codex>` — which agent CLI to burn against (default `claude`).
- `--model <id>` — model to pass through; default is the harness's own default.
  See `brulr models` for known ids (any id the harness accepts still works).
- `--effort <level>` — reasoning effort. claude: `low|medium|high|xhigh|max`;
  codex: `minimal|low|medium|high`. Default: the harness/model default.
- `--until <HH:MM>` — burn until the next occurrence of a local wall-clock time.

## How it works

Each call carries a fixed per-call overhead plus a block of random hex padding,
placed at the front so prefix caching can't absorb it. On start, brülr
**calibrates** with two probe calls to learn the per-call overhead and
tokens-per-byte, then sizes each call's padding to hit the target — trimming the
last call so it doesn't overshoot.

The end-of-run report separates **raw tokens** (everything at face value — the
leaderboard number) from **cost-weighted** tokens (cache reads discounted to
~0.1×, since that's what they actually cost). A warning fires if too much input
is being served from cache, meaning the burn isn't real.

## Library

The crate is also a library: implement the `Burner` trait for a new backend, or
call `calibrate` / `burn` directly. `brulr` (the binary) is a thin CLI over it.
