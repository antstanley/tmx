# Done Certificate — Task 27: Run store (`.tmx/runs`) and `tmx runs`

**Task:** [27-run_store.md](27-run_store.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06 — verdict DONE (attempt 3)

> This certificate is a verification protocol for Task 27. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 27) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Ship `LocalRunStore` persisting each run to `.tmx/runs/<uuidv7>/` (final-state snapshot + ndjson log) with the event-log cap and retention, plus `QueryRuns` behind `tmx runs`.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** The Task-26 event stream and final-state sink still stream to stdout/stderr unchanged (the store records from that stream, it does not replace it); the Task-09 masking guarantee still holds — the store persists the post-Masker payload, so a replay cannot re-expose a secret; Task-17 `tmx run` still prints normally with the store on or off.

## Obligations

- **O1 — A run persists to `.tmx/runs/<uuidv7>/`, `tmx runs list` shows runs chronologically, `state`/`logs` dump the masked snapshot and log, and `--no-store` records nothing.**
  - *Claim:* each run persists as a `RunRecord` (final-state snapshot + ndjson event log) under `.tmx/runs/<uuidv7>/`, keyed by the time-ordered UUIDv7 so listings are chronological without a sort key; `tmx runs list/show/state/logs` dump the masked snapshot and replay the masked log; `--no-store` opts out and writes nothing.
  - *Evidence to collect:* read `crates/tmx-adapters/src/runstore.rs` for the `LocalRunStore` write layout and the UUIDv7 keying; read `crates/tmx-cli/src/commands/runs.rs` for `list`/`show`/`state`/`logs`; confirm `.gitignore` lists `.tmx/runs/` as untracked; run the named test that persists runs then lists them chronologically and dumps `state`/`logs`; run a `--no-store` flow and confirm nothing is written under `.tmx/runs/`.
  - *Checks:* trace that `tmx runs state`/`logs` reads the payload the Task-26 sink already routed through the Masker (persist-after-mask), so a replay cannot re-expose a secret.
  - *Status:* ☑ SATISFIED — `LocalRunStore` writes `record.json` + `log.ndjson` under `<base>/<uuidv7>/` (`crates/tmx-adapters/src/runstore.rs`); `list()` lexically sorts UUIDv7 ids (chronological, no sort key). `.gitignore:13` carries `**/.tmx/runs/` and `git check-ignore crates/tmx-cli/.tmx/runs/foo` matches. Exercised live: two `tmx run flow.yaml` then `tmx runs list` listed both chronologically; `show`/`state`/`logs` dumped the record, final state, and `[run.start, task.start, task.finish, run.finish]`. `--no-store` run left no `.tmx` directory at all. Persist-after-mask traced: `StoringSink::emit` tees the reporter first then persists `event.get()` from the `Masked<Event>`; a live secret run's replayed log contained `[REDACTED]` and zero occurrences of the raw secret (`cli_runs::a_persisted_log_replays_with_secrets_masked` also passes).

- **O2 — At `EVENT_LOG_MAX_BYTES` the log writes `log.truncated` and stops persisting without aborting the run, and retention prunes an aged record.**
  - *Claim:* on the persisted log reaching `EVENT_LOG_MAX_BYTES` a final `log.truncated` event is written and persistence stops for that run while stdout streaming and the final-state snapshot continue; retention (default 30 days) prunes an aged record; the record is capped, never silently sampled.
  - *Evidence to collect:* run the test that drives the persisted log to `EVENT_LOG_MAX_BYTES` and confirm a final `log.truncated` is written, persistence stops, and the run still completes with its final-state snapshot intact; run the retention test that prunes an aged record (opportunistically at `tmx run` and via `tmx runs prune`, with `runs.retention`/`TMX_RUNS_RETENTION` and `0`/`off` disabling); confirm the log is truncated at the boundary rather than down-sampled.
  - *Status:* ☑ SATISFIED — `runstore::tests::the_event_log_is_capped_with_a_truncated_marker_and_then_stops` passes: at the cap exactly one `log.truncated` is written, later appends drop silently (`LogState.stopped`), and `save` still persists the snapshot afterwards. The cap decision truncates at the byte boundary (`bytes + event_bytes > cap` → stop), never samples. Retention exercised live: a seeded `startedAt: 2000-01-01` record pruned by `tmx runs prune` (`{"pruned": 1}`) and again opportunistically at the next `tmx run` ("tmx: pruned 1 run record(s)…"), while the fresh run survived; `TMX_RUNS_RETENTION=off` pruned 0 and `resolve_retention_with` unit tests cover `0`/`off`/garbage-falls-back-to-default.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; confirm `EVENT_LOG_MAX_BYTES` and the retention default are named units-last constants in `tmx-schema::limits`.
  - *Status:* ☑ SATISFIED — independently run from the main tree: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 339/339 passed (and left no `crates/tmx-cli/.tmx` behind — the attempt-2 pollution is fixed via `--no-store` in `cli_run.rs::run_flow_args`); `scripts/purity.sh` green. `EVENT_LOG_MAX_BYTES` (pre-existing) and the new `RUN_RETENTION_DEFAULT_DAYS: u64 = 30` are units-last named constants in `tmx-schema::limits` with a compile-time floor assertion. Zero `.tmx/runs` paths tracked by git.

- **O4 — Reviewable: run a flow, `tmx runs list` then `show`/`logs`, confirming chronological ordering, masked output, and a pruned aged record.**
  - *Claim:* a reviewer can run a flow, then `tmx runs list` (chronological by UUIDv7), then `tmx runs show`/`tmx runs logs` (masked snapshot and replayed masked log), and prune an aged record.
  - *Evidence to collect:* run a real flow; run `tmx runs list` and observe chronological ordering; run `tmx runs show` and `tmx runs logs` and observe the masked snapshot and log; seed an aged record and run `tmx runs prune` and confirm it is removed.
  - *Status:* ☑ SATISFIED — performed against the compiled binary in a temp dir: two real runs listed chronologically by UUIDv7; `show`/`state`/`logs` dumped the masked record, final state (`build.message = "built-ok"`), and the ordered event replay; a secret-echoing run's `logs`/`state` showed `[REDACTED]` and never the raw value; a seeded aged record was removed by `tmx runs prune` (`pruned: 1`) with the fresh run kept. Negative space also held: missing run → `Resolution [run_not_found]`, exit 4; malformed id → `Validation [invalid_run_id]`, exit 3; `rm` removed a run and a subsequent `state` exited 4; `runs list` over an empty store printed `{"runs": []}` with exit 0.

## Regression check

- Task-26 event stream: trace that a `tmx run --no-store flow.yaml` still prints the masked final state to stdout and the pretty progress to stderr exactly as a stored run does — disabling the store changes only persistence, not the reporter output path.

## Residue

- The chronological-without-sort-key claim rests on monotonic UUIDv7 generation; confirm two runs in the same millisecond still order correctly.
- Opportunistic retention runs on each `tmx run`; confirm a large `.tmx/runs/` does not add unbounded latency to an unrelated run, and that `0`/`off` fully disables the sweep.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: DONE — O1, O2, O3, O4 all SATISFIED.
CONFIDENCE: high
SUMMARY: `LocalRunStore` persists each run as record.json + log.ndjson under `./.tmx/runs/<uuidv7>/`, capped (one `log.truncated`, then stop) and retained (default 30 days via `RUN_RETENTION_DEFAULT_DAYS`, swept at `tmx run` and `tmx runs prune`, `0`/`off` disables); `QueryRuns` behind `tmx runs list/show/state/logs/prune/rm` replays only the post-Masker payload. Regression check held: `tmx run --no-store` prints the identical masked stdout/stderr as a stored run (store tee wraps, never replaces, the reporter). Residue: same-millisecond ordering rests on `uuid::Uuid::now_v7()` monotonicity (unit-tested in `idgen.rs`, pre-existing code); the opportunistic sweep is O(records) record.json reads, fully skipped when disabled. All four repo gates green (fmt, clippy -D warnings, nextest 339/339, purity), attempt-2's tracked/polluting `.tmx/runs` artifacts are gone and ignored via `**/.tmx/runs/`.
