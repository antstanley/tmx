# Done Certificate — Task 26: Reporters and the canonical event stream

**Task:** [26-reporters_and_events.md](26-reporters_and_events.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 26. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 26) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** Ship the three `EventSink` reporters (pretty → stderr, ndjson → stdout, final-state → stdout), `--format` selection, and the complete event stream, every payload masked at the boundary.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** The Task-17 final-state stdout reporter still yields the same masked JSON object under the default pipe path (this task replaces/extends it via `FinalStateSink`); the Task-09 Masker boundary assertion still fails closed; the Task-11/12 runner and Task-19 `eval` still emit their events, now widened to the full set.

## Obligations

- **O1 — `--format pretty|json|ndjson` selects the right stdout renderer, the full event stream is emitted in order, and stderr progress is independent of stdout data.**
  - *Claim:* `--format` selects `PrettySink` (human summary to stderr), `NdjsonSink` (one event per line to stdout), or `FinalStateSink` (the merged JSON object to stdout), with `pretty` the TTY default and `json` the pipe default; the full event set (`run.start`/`run.finish`, `task.start`/`task.finish`, `task.skip`, `task.error`, `map.item.finish`/`eval.case.finish`, `hook.start`/`hook.finish`, `log.truncated`) is emitted in order; stderr progress is independent of stdout data.
  - *Evidence to collect:* read `crates/tmx-cli/src/compose.rs` for the `--format` reporter selection and the TTY check; read `crates/tmx-adapters/src/sink/` for `PrettySink`/`NdjsonSink`/`FinalStateSink` and their target streams; read `crates/tmx-core/src/model.rs` for the `Event` set; run the named test asserting the ordered event stream and the stderr/stdout split.
  - *Status:* ☐ unverified

- **O2 — Every event and final-state payload is masked, and a sink that skips the Masker trips its assertion.**
  - *Claim:* every event and final-state payload is routed through the Masker before emission, so a secret in a task output never appears in any sink; a sink that emits without routing through the Masker trips a per-sink assertion in tests.
  - *Evidence to collect:* run the masking test that seeds a secret in a task output and asserts it appears redacted in each of the three sinks; run the negative test where a sink emits a payload bypassing the Masker and confirm the per-sink boundary assertion trips.
  - *Checks:* trace every sink emission through the Masker and confirm the per-sink assertion.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☐ unverified

- **O4 — Reviewable: one flow run three times under each `--format`, stdout diffed, confirming the event stream, the stream separation, and the masking.**
  - *Claim:* a reviewer can run one flow under `--format pretty`, `--format json`, and `--format ndjson`, diff the captured stdout, and observe the ordered event stream (ndjson), the final-state object (json), the pretty summary confined to stderr, and a seeded secret redacted in every case.
  - *Evidence to collect:* run the same flow under each of the three formats, capturing stdout and stderr separately; diff the three stdouts; grep each stream for the seeded secret and confirm redaction; confirm the pretty renderer wrote only to stderr.
  - *Status:* ☐ unverified

## Regression check

- Task-17 final-state reporter: trace that `tmx run flow.yaml | jq .` (json/pipe default) still emits the identical masked final-state JSON object it did before this task — the `FinalStateSink` is behaviour-preserving on the default path.

## Residue

- `NO_COLOR`/`--color`/`--no-color` and the TTY check are steps but not a DoD obligation; a validator may still confirm `NO_COLOR` disables colour on the `pretty`/`json` default.
- `log.truncated` is emitted here but its persistence cap belongs to Task 27; confirm the event is produced even when the run store is off.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
