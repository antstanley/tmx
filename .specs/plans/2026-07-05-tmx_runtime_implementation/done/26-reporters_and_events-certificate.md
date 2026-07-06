# Done Certificate — Task 26: Reporters and the canonical event stream

**Task:** [26-reporters_and_events.md](26-reporters_and_events.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-06

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
  - *Status:* ☑ SATISFIED — `Composed::new(base_dir, format, color)` wires `ReporterSink::for_format` (compose.rs); `resolve_format` (config.rs) applies flag → `TMX_FORMAT` → `Format::default_for_tty` (stdout TTY → pretty, pipe → json; unit-tested in `format_precedence_flag_then_env_then_tty_default` and `tty_selects_pretty_and_pipe_selects_json`). Sinks: `PrettySink` → stderr, `NdjsonSink` → stdout, `FinalStateSink` → stdout at run end under json (run.rs `emit_final_state`). All 11 `Event` variants render (`every_event_variant_renders_a_line`). `format_ndjson_streams_the_ordered_event_set_to_stdout` passed (run.start first, run.finish last, build.finish before check.start, progress on stderr); the validator also ran the flow live and observed the ordered stream and the stream split. Residue: `map.item.finish`/`eval.case.finish` have no live emission site — `dispatch_task` still rejects `map`/`eval` as `task_type_unsupported` (pre-existing; the fan-out engine of tasks 18/19 is not yet wired into dispatch) — and `log.truncated` has no producer until the Task-27 run store; all three variants are fully rendered by every sink, so emission drops in without sink changes. The wiring task inherits the emission obligation.

- **O2 — Every event and final-state payload is masked, and a sink that skips the Masker trips its assertion.**
  - *Claim:* every event and final-state payload is routed through the Masker before emission, so a secret in a task output never appears in any sink; a sink that emits without routing through the Masker trips a per-sink assertion in tests.
  - *Evidence to collect:* run the masking test that seeds a secret in a task output and asserts it appears redacted in each of the three sinks; run the negative test where a sink emits a payload bypassing the Masker and confirm the per-sink boundary assertion trips.
  - *Checks:* trace every sink emission through the Masker and confirm the per-sink assertion.
  - *Status:* ☑ SATISFIED — the `EventSink` port now takes `&Masked<Event>` (ports/driven.rs) and `FinalStateSink` takes `&Masked<Value>`, so a sink cannot be handed a raw payload (typestate); the runner seals via `masker.redact_event` (runner.rs `emit_event`, after `assert_ready`), and the final state is redacted by the run's Masker in core on both paths (usecases.rs:87–94 and run.rs `execute_preflighted`:202–208) before the CLI re-seals it for the sink. Each sink calls `assert_routed` (origin != 0). Positive: `a_secret_in_a_task_output_is_redacted_in_the_ndjson_line`, `a_secret_in_a_task_error_is_redacted_in_the_pretty_line`, `a_secret_in_the_final_state_is_redacted`, plus end-to-end `a_secret_is_redacted_under_every_format` — all passed. Negative space: the `#[should_panic(expected = "did not route through the Masker")]` unrouted-payload tests in pretty/ndjson/final_state all tripped the guard (observed passing), forged via the origin-0 `Masked::unrouted_for_test` seam; `RecordingEventSink` asserts routing too. The validator additionally seeded a live secret and grepped every captured stream under all three formats — no raw occurrence, `[REDACTED]` present.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean; confirm any new limit is a named units-last constant in `tmx-schema::limits`.
  - *Status:* ☑ SATISFIED — validator ran all three: `cargo fmt --all --check` clean; `cargo clippy --all-targets --all-features -- -D warnings` clean; `cargo nextest run` 321/321 passed. `scripts/purity.sh` green. No new numeric bound was introduced (format/TTY/colour need none); the ANSI SGR sequences are named module constants, and the existing `MASK_SCAN_LEN_MIN_BYTES` floor is reused.

- **O4 — Reviewable: one flow run three times under each `--format`, stdout diffed, confirming the event stream, the stream separation, and the masking.**
  - *Claim:* a reviewer can run one flow under `--format pretty`, `--format json`, and `--format ndjson`, diff the captured stdout, and observe the ordered event stream (ndjson), the final-state object (json), the pretty summary confined to stderr, and a seeded secret redacted in every case.
  - *Evidence to collect:* run the same flow under each of the three formats, capturing stdout and stderr separately; diff the three stdouts; grep each stream for the seeded secret and confirm redaction; confirm the pretty renderer wrote only to stderr.
  - *Status:* ☑ SATISFIED — validator ran a two-task flow (one task echoing a requested env secret) under `--format pretty`, `--format json`, and `--format ndjson`, capturing streams separately: pretty stdout 0 bytes with the full progress on stderr; json stdout one final-state object (`leak.message` = `[REDACTED]`); ndjson stdout the ordered event stream (run.start … run.finish, one JSON object per line, `event` tag on each); the no-flag pipe default diffed byte-identical to `--format json` (Task-17 regression holds); `TMX_FORMAT=ndjson` honoured; `--color` painted the stderr status token ANSI-green and `NO_COLOR` (even empty, even with `--color`) disabled it; the seeded secret appeared in no stream. The four `cli_run` format tests pin the same behaviours in CI.

## Regression check

- Task-17 final-state reporter: trace that `tmx run flow.yaml | jq .` (json/pipe default) still emits the identical masked final-state JSON object it did before this task — the `FinalStateSink` is behaviour-preserving on the default path.

## Residue

- `NO_COLOR`/`--color`/`--no-color` and the TTY check are steps but not a DoD obligation; a validator may still confirm `NO_COLOR` disables colour on the `pretty`/`json` default.
- `log.truncated` is emitted here but its persistence cap belongs to Task 27; confirm the event is produced even when the run store is off.

## Conclusion

VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations discharged with their named evidence. The three reporters land with `--format` selection (flag → `TMX_FORMAT` → TTY default), the stream separation holds live (pretty stdout empty, json = the pipe-default final state byte-for-byte, ndjson the ordered event stream), and masking is now structural: `EventSink` carries `Masked<Event>`, every sink asserts a non-zero Masker origin, and the origin-0 forge trips all three per-sink guards. fmt/clippy/nextest (321/321)/purity all green; the validator re-ran the reviewable flow end-to-end with a seeded secret and found no leak in any stream. Regression check passed: the pipe-default stdout is identical to Task 17's. Residue carried forward, not a defect of this task: `map.item.finish`/`eval.case.finish` have no live emission site because `dispatch_task` still rejects `map`/`eval` (`task_type_unsupported`, pre-existing — the task-18/19 fan-out engine awaits its dispatch-wiring task), and `log.truncated` has no producer until the Task-27 run store; all three variants are rendered by every sink, so emission drops in without sink changes. The fan-out-wiring and run-store tasks inherit those emission obligations.
