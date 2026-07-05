# Task 27 — Run store (`.tmx/runs`) and `tmx runs`

**Plan:** [plan.md](../plan.md) · **Certificate:** [27-run_store-certificate.md](27-run_store-certificate.md)

**Implements:** [08-errors-and-observability.md](../../../08-errors-and-observability.md) §Run store; [07-cli.md](../../../07-cli.md) §Pipeline runs; [06-ports-and-adapters.md](../../../06-ports-and-adapters.md) §Cross-cutting driven ports (`RunStore`)
**Depends on:** 17, 26
**Produces:** `LocalRunStore` persisting each run to `.tmx/runs/<uuidv7>/` (final-state snapshot + ndjson log) with the event-log cap and retention, plus `QueryRuns` behind `tmx runs`
**Pointers:** `crates/tmx-adapters/src/runstore.rs` (new), `crates/tmx-cli/src/commands/runs.rs` (new), `.gitignore` (confirm `.tmx/runs/` untracked)

## Steps

- [ ] Persist each run as a `RunRecord` — a final-state snapshot plus the ndjson event log — under `.tmx/runs/<uuidv7>/`, keyed by the time-ordered UUIDv7 so listings are chronological without a sort key.
- [ ] Cap the persisted log by `EVENT_LOG_MAX_BYTES`: on overflow write a final `log.truncated` event and stop persisting for that run, while stdout streaming and the final-state snapshot continue.
- [ ] Implement retention (default 30 days, opportunistic at each `tmx run` and on demand via `tmx runs prune`; `runs.retention`/`TMX_RUNS_RETENTION`, `0`/`off` disables) and `--no-store` to opt out.
- [ ] Implement `QueryRuns` behind `tmx runs list/show/state/logs/prune/rm`, dumping the masked final state and replaying the masked event log.

## Definition of done

- [ ] A run persists to `.tmx/runs/<uuidv7>/` and `tmx runs list` shows runs chronologically; `state`/`logs` dump the masked snapshot and log; `--no-store` records nothing.
- [ ] An event log reaching `EVENT_LOG_MAX_BYTES` writes `log.truncated` and stops persisting without aborting the run, and retention prunes an aged record (negative space: the record is capped, never silently sampled).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run a flow, `tmx runs list` then `tmx runs show`/`logs`, and confirm chronological ordering, masked output, and a pruned aged record.
