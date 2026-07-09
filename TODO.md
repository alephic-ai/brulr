# brülr TODO

## Core burn engine

- [ ] **More backends.** The `Burner` trait, `claude` one-shot, and `codex`
      backends are done. Still to add: raw Anthropic/Bedrock API (fleet
      load-generator) and claude streaming.
      - Claude streaming flags: `-p --verbose --output-format stream-json
      --include-partial-messages --disallowedTools AskUserQuestion`.
      Auto-inject and *normalize* existing values rather than blindly
      appending.
- [ ] **Burn-rate strategy.** `--strategy fastest-burn-rate` (tokens/sec) vs
      `cheapest-per-token`. (`--model` passthrough already exists.)

## Burn algorithms

- [ ] **Task-based entropy variation.** The random-hex padding is done,
      front-loaded so prefix caching dies on the first divergent token, and the
      cache-counter check already warns when entropy is absorbed. Rotating
      rng-parameterized tasks (bounded ~0.5-2k output tokens, single-turn to
      keep tool-use cache re-reads out) now exist in `rng::build_task`. Still
      to do: vary the output format (prose, table, bullets, Q&A) and framing.
- [ ] **Thinking-token amplification.** Set `MAX_THINKING_TOKENS=128000` (on the
      claude command, not before a pipe) and seed problems that demand "show
      every arithmetic operation." A message-derived deterministic seed gives
      the same problems every time, so seed from urandom to diverge across
      invocations and rotate the size tier (1/3/5/10 problems).
- [ ] **Problem bank.** Exhaustive enumeration beats hard math for token volume.
      Strong burners: 4096-mask subset-sum sweep, 720-tour TSP enumeration, 5x5
      cofactor determinant recursion, 30-digit long division, Floyd-Warshall
      printing all 7 matrices, Gaussian elimination in exact fractions.
      Generate parameterized instances in Rust; don't ship a static bank.
      (A first slice exists: the four simple tasks in `rng::build_task`.)
- [ ] **Output-token maximization mode.** Demand 2000-word-plus responses, and
      full-file reads into subagent context (input burn) with verbose analysis
      (output burn). Ship code task templates (architecture forensics,
      line-by-line security audit, complexity heatmap, and so on) as prompt-pack
      TOML.
- [ ] **Recursive summarization ouroboros.** Generate, summarize, expand,
      summarize again. Zero information gain by construction.
- [ ] **Subagent fan-out.** Parallel workers multiply burn linearly.
      See the queue design below.

## Parallel execution

- [ ] **Atomic file-claim queue.** Tasks are `pending-N` files, and a worker
      claims one with `mv pending-N claimed-N`, which atomically succeeds for
      exactly one worker. No locks, no daemon, and you can debug it with `ls`.
- [ ] **Stop-file drain.** A single touched stop file that workers check
      before claiming the next task, never mid-task. Graceful drain for
      free. Rate-limit auto-stop, Ctrl-C, and deadline all use the
      same mechanism.
- [ ] **Cancel-flag hygiene** *(subtle bug worth guarding against)*.
      Reset the CANCELLED flag before each task. Otherwise a SIGINT during
      task N mislabels task N+1's ordinary failure as "cancelled" and drops
      the error record.
- [ ] **Sidecar lock for state.** `state.json` guarded by an fs2 exclusive
      lock on `.state.json.lock`, re-read after acquiring the lock. Parallel
      workers, one state file, no torn writes.

## Result handling

- [ ] **Classify from stream-json.** Parse the last `"type":"result"`
      event: Success, RateLimited, Retryable (5xx), or Failed, mapped to
      exit codes 0/2/3/1. Edge cases: missing JSONL counts as Success, but
      *unreadable* JSONL counts as Failed, so an I/O error never records
      success.
- [ ] **Retry policy.** Retrying once and aborting on two consecutive failures
      is fine for a toy. brülr should distinguish Retryable (backoff and retry)
      from Failed (skip task) from RateLimited (touch stop file).

## Quota intelligence

- [ ] **Rate-limit event stream.** Parse `rate_limit_event` from stream-json
      live, with statuses `allowed`, `allowed_warning` (carrying a
      `utilization` float and `surpassedThreshold`), and `rejected`. A threshold
      breach touches the stop file.
- [ ] **Dual-window usage gate.** After each task, check used_percent for both
      the 5h and weekly windows and gate on the larger (the safe side). Run the
      usage check in the agent's env (CLAUDE_CONFIG_DIR and friends), or a
      multi-account setup reads the wrong account's quota.
- [ ] **Reset-deadline scheduler.** Fixed weekly reset (weekday/time/tz)
      computed with a real tz database (chrono-tz) and a DST-safe
      local-datetime resolver that handles the ambiguous and skipped hour
      cases explicitly. Also support deriving the deadline from the usage API's
      `resets_at`, picking the nearest of 5h vs weekly.
- [ ] **`brulr status`.** Burn-up chart vs linear pace line, `--compact` for
      tmux status bars, `--json` for scripting.

## Useful-waste mode

- [ ] **Repo sweep.** Auto-discover git repos, filter by remote-URL username,
      dedup across scan sources, and fetch GitHub visibility to sort public
      repos first, so you burn where the world can see the commits, or rather
      where nothing sensitive leaks into logs. Skip-within window keyed on
      per-(agent, directory) timestamps in state.json.
- [ ] **Prompt packs.** TOML task templates. Ship starter packs of code tasks
      and math categories.
- [ ] **Enforced read-only.** A prompt-only "reads and analyzes, never
      modifies" claim is enforced by nothing. brülr should use
      `--disallowedTools`, `--permission-mode plan`, and a PreToolUse deny hook,
      belt and braces.

## Theater

- [ ] **ANSI fire animation.** Hand-rolled frames, `tput sc`/`rc` to save and
      restore the cursor, a redraw of a fixed region above the progress bar,
      `\033[K` to clear stale line tails, and hide/show cursor with a trap on
      EXIT so a Ctrl-C never leaves the cursor invisible. Braille-cell flames,
      intensity proportional to tokens/sec.
- [ ] **Bless mode.** 10 sutras, 108x/1080x recitations with varied layouts
      (columns, spirals, grids, where layout variation doubles as cache
      defeat), 3 alternating ASCII Buddha frames, and a merit-dedication footer.
      `brulr bless <name>`, with a secular `brulr toast` variant.
- [ ] **End-of-run report flourishes.** Calls, raw and cost-weighted tokens, and
      USD are reported (claude via `total_cost_usd`, codex via the `CODEX_PRICES`
      table). Still missing: a duration line, "about N engineers (YC math)",
      and a staleness warning on the hardcoded codex rates.

## Leaderboard / metrics

- [ ] **Leaderboard submission.** Usage lands in local JSONL logs anyway, so
      `brulr flex` submits it to a leaderboard.
- [ ] **OTel export.** Tag `brulr.mode=waste` honestly. Dashboards can filter,
      and auditors can laugh.

## Safety rails

- [ ] **Hard budget cap.** `--max-usd` / `--max-tokens` (cost-weighted),
      mandatory in config, low default. (A `burn 5usd` dollar *target* exists;
      this is the separate safety *ceiling* that combines with any target.)
- [ ] **Dry run and cost estimate.** Calibration already measures per-call
      overhead and tokens/byte. Add tokens/sec and $/1M, plus a pre-burn estimate
      (a dry run that burns nothing).
- [ ] **Kill switch.** SIGINT touches the stop file and drains; `brulr stop
      --now` hard-kills panes. Keep a state file so a crash never orphans agents.

## Clients

- [ ] **Desktop version.** Ship a desktop app (tray/menubar or small window)
      that runs burns without the terminal: pick harness/model/target, show
      live progress and the end-of-run report, reuse the library `Burner` /
      `calibrate` / `burn` loop rather than reimplementing the engine.
