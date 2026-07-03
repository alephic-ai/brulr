# brülr
A CLI for burning AI tokens on purpose.

brülr runs an agent harness (`claude` or `codex`) in a loop and pads every call
with uncacheable random bytes. It burns toward whatever you give it: a token
count, a duration, a wall-clock time, or a dollar amount.

## Install

```sh
cargo build --release
```

The binary lands in `target/release/brulr`. You also need whichever harness you
burn against (`claude` and/or `codex`) installed and logged in.

## Usage

```sh
brulr burn                 # burn 100000 tokens (default), via claude
brulr burn 500000          # burn a token count
brulr burn 45m             # burn for a duration (90s, 45m, 2h)
brulr burn 5usd            # burn until $5 of API-equivalent cost
brulr burn --until 07:00   # burn until the next local 07:00

brulr burn --harness codex # burn via codex instead of claude
brulr burn --model claude-opus-4-8 --effort high
brulr models               # list known models per harness
```

Run `brulr burn --help` for all flags.

### Options

- `<target>`: what to burn toward. A token count (`100000`), a duration
  (`90s`/`45m`/`2h`), or a dollar amount (`5usd`/`0.25usd`). Defaults to `100000`.
- `--harness <claude|codex>`: which agent CLI to burn against. Defaults to `claude`.
- `--model <id>`: model to pass through. Defaults to the harness's own default.
  Run `brulr models` for known ids; any id the harness accepts still works.
- `--effort <level>`: reasoning effort. claude takes `low|medium|high|xhigh|max`,
  codex takes `minimal|low|medium|high`. Defaults to the harness/model default.
- `--until <HH:MM>`: burn until the next occurrence of a local wall-clock time.

## How it works

Every call pays a fixed per-call overhead and then carries a block of random hex
padding. The padding sits at the front of the prompt so prefix caching can't
absorb it. At startup brülr makes two probe calls to measure the overhead and
the tokens-per-byte rate, then sizes each call's padding to reach the target. It
trims the last call so the run doesn't overshoot.

The end-of-run report gives two token totals. Raw tokens count everything at
face value, which is the number you'd quote on a leaderboard. Cost-weighted
tokens discount cache reads to about a tenth, since that is closer to what they
actually cost. If too much of the input is being served from cache, the run
prints a warning: the padding is being cached and the burn isn't real.

### Cost

The report also prints a dollar figure, and `burn 5usd` burns until it hits a
target spend. `claude` reports its own cost, so those numbers are exact. `codex`
doesn't, so its cost comes from a hardcoded price snapshot (`CODEX_PRICES` in
`src/lib.rs`); check it against current pricing before you trust the codex
dollars. On a subscription these are API-equivalent dollars, not charges against
your plan. On a metered API key it would be real money.

## Library

The crate is also a library. Implement the `Burner` trait to add a backend, or
call `calibrate` and `burn` yourself. The `brulr` binary is a thin CLI on top.
