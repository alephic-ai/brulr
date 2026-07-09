# brülr

A CLI for burning AI tokens on purpose.

![brülr feeding tokens into a furnace while a leaderboard applauds](assets/brulr-satire-v3.png)

brülr runs an agent harness (`claude`, `codex`, or `grok`) in a loop and pads
every call with uncacheable random bytes. It burns toward whatever you give it:
a token count, a duration, a wall-clock time, or a dollar amount.

## Why

Some companies now measure how well people are "adopting AI" by counting the
tokens they burn. There are dashboards for it. They rank everyone, put the
biggest spenders at the top as power users, and mark anyone with little or no
spend as inactive or as a coaching opportunity.

But burning tokens is not the same as doing work. Solve something in one good
prompt, use a cheaper model, or just not need the thing this week, and you look
identical to someone who did nothing. So the metric quietly rewards waste, and
the person being careful with it comes off worse than the
[Token Maximalist](https://www.alephic.com/store/token-maximalist-oversized-faded-t-shirt-1)
spraying tokens at everything.

brülr is what happens if you take that metric at its word. If the score is only
consumption, you can win it without doing anything, so brülr does exactly that:
tokens in, nothing out. Point it at your quota and it climbs the leaderboard
while producing nothing at all. If a program this dumb can top your chart, the
chart was never measuring what you thought it was.

## Install

```sh
brew install ubi
ubi --project alephic-ai/brulr --in ~/.local/bin
```

The first line installs [ubi](https://github.com/houseabsolute/ubi), a small
tool that downloads prebuilt binaries from GitHub releases. The second line
fetches the latest brülr release, picks the build matching your OS and CPU, and
drops the `brulr` binary into `~/.local/bin`. Make sure that directory is on
your `PATH`.

No Rust toolchain needed. If you do want to build from source, `cargo build
--release` puts the binary at `target/release/brulr`.

You still need whichever harness you burn against (`claude`, `codex`, and/or
`grok`) installed and logged in.

## Usage

```sh
brulr burn                 # burn 100000 tokens (default), via claude
brulr burn 500000          # burn a token count
brulr burn 45m             # burn for a duration (90s, 45m, 2h)
brulr burn 5usd            # burn until $5 of API-equivalent cost
brulr burn --until 07:00   # burn until the next local 07:00

brulr burn --harness codex # burn via codex instead of claude
brulr burn --harness grok  # burn via the xAI Grok Build CLI
brulr burn --model claude-opus-4-8 --effort high
brulr models               # list known models per harness
```

Run `brulr burn --help` for all flags.

### Options

- `<target>`: what to burn toward. A token count (`100000`), a duration
  (`90s`/`45m`/`2h`), or a dollar amount (`5usd`/`0.25usd`). Defaults to `100000`.
- `--harness`, `--model`, `--effort`: see [Harnesses, models, and
  efforts](#harnesses-models-and-efforts) below.
- `--until <HH:MM>`: burn until the next occurrence of a local wall-clock time.

### Harnesses, models, and efforts

brülr shells out to an agent CLI (the **harness**), optionally with a
**model** and **reasoning effort**. Known models must match their harness;
unknown model ids still pass through. Effort is validated for the selected
model (or the harness default when `--model` is omitted). Mismatches fail fast
(exit 2), e.g. `--model grok-4.5` without `--harness grok`.

Defaults: `--harness claude`; omit `--model` / `--effort` for the harness
defaults. Run `brulr models` (or `brulr models --harness grok`) to print the
known-model snapshot. Source of truth: `src/catalog.rs` (will go stale; any id
the harness still accepts works even if it is not listed).

| `--harness` | CLI | Install / login |
| --- | --- | --- |
| `claude` (default) | `claude` | Claude Code, logged in |
| `codex` | `codex` | OpenAI Codex CLI, logged in |
| `grok` | `grok` | [Grok Build CLI](https://x.ai/cli), logged in |

#### `claude`

| | |
| --- | --- |
| **Efforts** | `low` · `medium` · `high` · `xhigh` · `max` |
| **Models** | `claude-sonnet-5` · `claude-fable-5` · `claude-opus-4-8` · `claude-opus-4-7` · `claude-sonnet-4-6` · `claude-opus-4-6` · `claude-opus-4-5-20251101` · `claude-haiku-4-5-20251001` · `claude-sonnet-4-5-20250929` · `claude-opus-4-1-20250805` |

All listed Claude models share the same effort set.

#### `codex`

| | |
| --- | --- |
| **Efforts** | `minimal` · `low` · `medium` · `high` |
| **Models** | `gpt-5.3-codex` · `gpt-5.2-codex` · `gpt-5.1-codex-max` · `gpt-5.1-codex-mini` · `gpt-5.1-codex` · `gpt-5-codex` |

All listed Codex models share the same effort set.

#### `grok`

Effort is **per model** (not shared across the harness):

| Model | Efforts |
| --- | --- |
| `grok-4.5` (default) | `minimal` · `low` · `medium` · `high` · `xhigh` · `max` |
| `grok-composer-2.5-fast` | — (`--effort` is rejected) |

## How it works

Every call pays a fixed per-call overhead and then carries a block of random hex
padding. The padding sits at the front of the prompt so prefix caching can't
absorb it. Each prompt ends with a rotating, randomly parameterized busywork
task (integers in English words, multiplication tables, hex conversions, digit
sums) sized to burn roughly 500–2000 output tokens per call — output is priced
several times higher than input, so the reply burns too. At startup brülr makes
two probe calls to measure the overhead and the tokens-per-byte rate, then sizes
each call's padding to reach the target. It trims the last call so the run
doesn't overshoot. The probes ask for a minimal fixed reply instead of a task,
so output variance can't skew the measurement.

The end-of-run report gives two token totals. Raw tokens count everything at
face value, which is the number you'd quote on a leaderboard. Cost-weighted
tokens discount cache reads to about a tenth, since that is closer to what they
actually cost. If too much of the input is being served from cache, the run
prints a warning: the padding is being cached and the burn isn't real.

### Cost

The report also prints a dollar figure, and `burn 5usd` burns until it hits a
target spend. `claude` reports its own cost, so those numbers are exact. `codex`
and `grok` don't, so their cost comes from hardcoded price snapshots
(`CODEX_PRICES` / `GROK_PRICES` in `src/catalog.rs`); check them against current
pricing before you trust those dollars. Grok also omits token counts from
headless JSON, so brülr recovers usage from the Grok Build log
(`~/.grok/logs/unified.jsonl`). On a subscription these are API-equivalent
dollars, not charges against your plan. On a metered API key it would be real
money.

## Library

The crate is also a library. Implement the `Burner` trait to add a backend, or
call `calibrate` and `burn` yourself. The `brulr` binary is a thin CLI on top.
