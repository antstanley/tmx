# Task 26 — Reporters and the canonical event stream

**Plan:** [plan.md](../plan.md) · **Certificate:** [26-reporters_and_events-certificate.md](26-reporters_and_events-certificate.md)

**Implements:** [08-errors-and-observability.md](../../../08-errors-and-observability.md) §Events & reporters, §Masking at the boundary; [07-cli.md](../../../07-cli.md) §stdout / stderr contract
**Depends on:** 12, 17, 19
**Produces:** the three `EventSink` reporters (pretty → stderr, ndjson → stdout, final-state → stdout), `--format` selection, and the complete event stream, every payload masked at the boundary
**Pointers:** `crates/tmx-adapters/src/sink/` (new, Pretty/Ndjson/FinalState), `crates/tmx-cli/src/compose.rs` (reporter selection), `crates/tmx-core/src/model.rs` (`Event`)

## Steps

- [ ] Emit the full event set from the runner: `run.start`/`run.finish`, `task.start`/`task.finish`, `task.skip`, `task.error`, `map.item.finish`/`eval.case.finish`, `hook.start`/`hook.finish`, `log.truncated`.
- [ ] Implement the three sinks — `PrettySink` (human summary to stderr), `NdjsonSink` (one event per line to stdout), `FinalStateSink` (the merged JSON object to stdout) — and select the stdout reporter by `--format` (`pretty` TTY default, `json` pipe default, `ndjson`), keeping stderr progress independent.
- [ ] Route every event and final-state payload through the Masker before emission, and add the per-sink assertion that a payload routed through the Masker (the negative-space guarantee that a new sink cannot bypass masking).
- [ ] Honour `NO_COLOR`/`--color`/`--no-color` and the TTY check for the `pretty`/`json` default.

## Definition of done

- [ ] `--format pretty|json|ndjson` selects the right stdout renderer, the full event stream is emitted in order, and stderr progress is independent of stdout data.
- [ ] Every event and final-state payload is masked (a secret in a task output never appears in any sink), and a sink that skips the Masker trips its assertion in tests (negative space).
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run one flow three times under each `--format`, diff the stdout, and confirm the event stream, the stream separation, and the masking.
