# Done Certificate — Task 27: Run store (`.tmx/runs`) and `tmx runs`

**Task:** [27-run_store.md](27-run_store.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

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
  - *Status:* ☐ unverified

- **O2 — At `EVENT_LOG_MAX_BYTES` the log writes `log.truncated` and stops persisting without aborting the run, and retention prunes an aged record.**
  - *Claim:* on the persisted log reaching `EVENT_LOG_MAX_BYTES` a final `log.truncated` event is written and persistence stops for that run while stdout streaming and the final-state snapshot continue; retention (default 30 days) prunes an aged record; the record is capped, never silently sampled.
  - *Evidence to collect:* run the test that drives the persisted log to `EVENT_LOG_MAX_BYTES` and confirm a final `log.truncated` is written, persistence stops, and the run still completes with its final-state snapshot intact; run the retention test that prunes an aged record (opportunistically at `tmx run` and via `tmx runs prune`, with `runs.retention`/`TMX_RUNS_RETENTION` and `0`/`off` disabling); confirm the log is truncated at the boundary rather than down-sampled.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; confirm `EVENT_LOG_MAX_BYTES` and the retention default are named units-last constants in `tmx-schema::limits`.
  - *Status:* ☐ unverified

- **O4 — Reviewable: run a flow, `tmx runs list` then `show`/`logs`, confirming chronological ordering, masked output, and a pruned aged record.**
  - *Claim:* a reviewer can run a flow, then `tmx runs list` (chronological by UUIDv7), then `tmx runs show`/`tmx runs logs` (masked snapshot and replayed masked log), and prune an aged record.
  - *Evidence to collect:* run a real flow; run `tmx runs list` and observe chronological ordering; run `tmx runs show` and `tmx runs logs` and observe the masked snapshot and log; seed an aged record and run `tmx runs prune` and confirm it is removed.
  - *Status:* ☐ unverified

## Regression check

- Task-26 event stream: trace that a `tmx run --no-store flow.yaml` still prints the masked final state to stdout and the pretty progress to stderr exactly as a stored run does — disabling the store changes only persistence, not the reporter output path.

## Residue

- The chronological-without-sort-key claim rests on monotonic UUIDv7 generation; confirm two runs in the same millisecond still order correctly.
- Opportunistic retention runs on each `tmx run`; confirm a large `.tmx/runs/` does not add unbounded latency to an unrelated run, and that `0`/`off` fully disables the sweep.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
