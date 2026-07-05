# Done Certificate — Task 06: Testkit fake adapters

**Task:** [06-testkit_fakes.md](06-testkit_fakes.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 06. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 06) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion.

## Premises

- **P1 — Goal.** One in-memory fake per driven port plus a fake-bundle constructor, so the core, the conformance suite, and downstream embedders inject one shared deterministic port set.
- **P2 — Obligations.** Done iff O1…O4 all hold; O2 is the negative-space item, O4 is the Reviewable item.
- **P3 — Invariants.** Task 05's driven port traits (`ProcessRunner`, `HttpClient`, `FileSystem`, `ObjectStore`, `ChatModel`, `SecretResolver`, `EnvironmentProvider`, `RunStore`, `EventSink`, `SourceLoader`, `ReferenceResolver`, `SchemaValidator`, `Clock`, `IdGenerator`, `Scheduler`). Each fake implements one of these exact trait signatures; that surface must remain intact. The consumers — the pipeline runner (Task 11) and conformance suite (Task 32) — are not yet built.

## Obligations

- **O1 — The fake bundle satisfies every driven port trait and is injectable into a use case; the `cargo tree` purity gate confirms `tmx-testkit` pulls in no async runtime or I/O crate.**
  - *Claim:* the fake-bundle constructor assembles one fake per driven port, each satisfying its Task 05 trait, injectable into a use case, and `tmx-testkit`'s tree contains no async runtime or I/O crate.
  - *Evidence to collect:* read the planned `crates/tmx-testkit/src/lib.rs` and the per-port modules `{scheduler,clock,idgen,process,http,chat,fs,store,sink}.rs`. Confirm each driven port from `ports::driven` has a fake impl (`SerialScheduler`, `FixedClock`, `SeededIdGenerator`, `RecordingProcessRunner`, `FakeHttpClient`, `FakeChatModel`, `MemFileSystem`, `MemObjectStore`, `RecordingEventSink`, and a fake `SecretResolver`/`RunStore`/`SourceLoader` as the suite needs), and that the fake-bundle constructor returns the full set. Confirm a test injects the bundle into a driving use case and it type-checks. Run the `cargo tree` purity check and confirm `tmx-testkit` depends on `tmx-core` + `tmx-schema` only — no `tokio`, no `reqwest`, no I/O crate.
  - *Checks:* resolve that each fake's `impl` block targets the `tmx-core` driven port trait (e.g. `impl Scheduler for SerialScheduler`), not a same-named trait declared locally in `tmx-testkit`.
  - *Status:* ☐ unverified

- **O2 — `SerialScheduler` returns results in index order for a shuffled completion order, and `FixedClock`/`SeededIdGenerator` produce identical sequences across two runs.**
  - *Claim:* `SerialScheduler` yields results in submission-index order regardless of completion order (a non-index-ordered result fails the test), and `FixedClock` and `SeededIdGenerator` emit identical sequences on repeat.
  - *Evidence to collect:* run the `tmx-testkit` unit tests. For `SerialScheduler`, confirm a test submits work whose completion order is shuffled and asserts the results emerge in index order, with a companion assertion that a non-index-ordered expectation fails. Confirm `SerialScheduler` asserts `concurrency >= 1` and in-flight `<= concurrency`. For `FixedClock`/`SeededIdGenerator`, confirm a test runs each twice and asserts byte-identical sequences.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run the `cargo tree` purity check and confirm `tmx-testkit` stays free of any async runtime or I/O crate. Task 06 changes no schema/example, so `scripts/validate.sh` is not required by this task.
  - *Status:* ☐ unverified

- **O4 — Reviewable: construct the fake bundle in a throwaway test, run it twice, and confirm byte-identical event streams and ids.**
  - *Claim:* a reviewer can construct the fake bundle in a throwaway test, run it twice, and observe byte-identical event streams and ids.
  - *Evidence to collect:* write and run a throwaway test that builds the fake bundle, drives it twice through the same sequence, and diffs the `RecordingEventSink` event stream and the `SeededIdGenerator` ids across the two runs — expect byte-identical output.
  - *Status:* ☐ unverified

## Regression check

- Task 06 implements the Task 05 driven port traits. Trace that each fake's method signatures still match their Task 05 trait definition (`SerialScheduler`→`Scheduler`, `RecordingEventSink`→`EventSink`, `FixedClock`→`Clock`, `SeededIdGenerator`→`IdGenerator`, …) and that Task 05's traits still compile with the core I/O-free : ☐ (PRESERVED / REGRESSION). The runner/conformance consumer is not yet built.

## Residue

- Task 06 step 3 says `RecordingEventSink` "asserts it routed every payload through the Masker," but the Masker is Task 09 — not yet built at Task 06. Treat the Masker-routing assertion as a forward reference (stubbed or deferred); do not fail O1/O2 on its absence, but note it as carried-forward work.
- The fake `SecretResolver`/`RunStore`/`SourceLoader` are provided "as the suite needs," so their presence is scoped to current consumers — a missing fake for a port with no consumer yet is acceptable if the bundle still type-checks for injected use cases.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
