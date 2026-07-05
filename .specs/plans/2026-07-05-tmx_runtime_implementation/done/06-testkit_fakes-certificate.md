# Done Certificate — Task 06: Testkit fake adapters

**Task:** [06-testkit_fakes.md](06-testkit_fakes.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-05

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
  - *Status:* ☑ SATISFIED — `Fakes` (lib.rs:59-90) has one field per driven port; all 15 fakes read (`SerialScheduler`/`FixedClock`/`SeededIdGenerator`/`RecordingProcessRunner`/`FakeHttpClient`/`FakeChatModel`/`MemFileSystem`/`MemObjectStore`/`MemRunStore`/`RecordingEventSink`/`FakeSecretResolver`/`FakeSourceLoader`/`FakeReferenceResolver`/`FakeSchemaValidator`/`FakeEnvironmentProvider`), and every `impl` block targets the `tmx_core::ports::driven` trait (each module `use`s the core trait; no local trait declared). `the_bundle_injects_into_a_use_case_and_records_the_run` injects `ids`/`event_sink`/`process` into a generic use case (bounds `IdGenerator`/`EventSink`/`ProcessRunner`) and passes; `every_non_generic_driven_port_is_object_safe_as_dyn` boxes each non-generic port as `dyn` (Scheduler correctly excluded — generic method). `scripts/purity.sh` → "✓ purity: tmx-schema, tmx-core, tmx-testkit carry no I/O or async dependency edge" (added deps async-trait/serde_json/indexmap are outside the gate's forbidden set; Cargo.lock adds no new transitive package).

- **O2 — `SerialScheduler` returns results in index order for a shuffled completion order, and `FixedClock`/`SeededIdGenerator` produce identical sequences across two runs.**
  - *Claim:* `SerialScheduler` yields results in submission-index order regardless of completion order (a non-index-ordered result fails the test), and `FixedClock` and `SeededIdGenerator` emit identical sequences on repeat.
  - *Evidence to collect:* run the `tmx-testkit` unit tests. For `SerialScheduler`, confirm a test submits work whose completion order is shuffled and asserts the results emerge in index order, with a companion assertion that a non-index-ordered expectation fails. Confirm `SerialScheduler` asserts `concurrency >= 1` and in-flight `<= concurrency`. For `FixedClock`/`SeededIdGenerator`, confirm a test runs each twice and asserts byte-identical sequences.
  - *Status:* ☑ SATISFIED — `serial_scheduler_returns_index_order_for_a_shuffled_completion_order` runs `make(i)=shuffled[i]` and asserts `values == [40,10,30,0,20]` (index order) plus `assert_ne!(values, sorted)` proving index order ≠ completion order. Negative-space verified live: injecting `out.reverse()` into `run_indexed` made this test FAIL (`left: [20,0,30,10,40], right: [40,10,30,0,20]` at lib.rs:249), then reverted — the guard is not vacuous. scheduler.rs:48-53 asserts `concurrency >= 1` and `concurrency <= CONCURRENCY_MAX`; scheduler.rs:59-63 asserts in-flight `<= concurrency`; `serial_scheduler_rejects_zero_concurrency` (`#[should_panic]`) passes. `fixed_clock_and_seeded_ids_produce_identical_sequences_across_two_runs` passes (two fresh clocks/generators emit identical sequences; ids also proven distinct). All within `cargo nextest run` = 46/46 passed.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo fmt --all --check` — expect all clean. Run the `cargo tree` purity check and confirm `tmx-testkit` stays free of any async runtime or I/O crate. Task 06 changes no schema/example, so `scripts/validate.sh` is not required by this task.
  - *Status:* ☑ SATISFIED — `cargo nextest run` → 46 tests, 46 passed, 0 skipped (15 new in tmx-testkit); `cargo clippy --all-targets --all-features -- -D warnings` → exit 0, no warnings; `cargo fmt --all --check` → clean; `scripts/purity.sh` → pass. Named-constant discipline upheld: `CONCURRENCY_MAX` reused from `tmx_schema::limits` (scheduler.rs:13,51), no new engine limit; local fixtures (`DEFAULT_SEED`, `DEFAULT_ORIGIN_UNIX_MS`, `TIMESTAMP_MASK_48`, `DEFAULT_INSTANT_RFC3339`, `DEFAULT_ORIGIN_MS`) are named with docs justifying local residence, matching the `model.rs` `RUN_ID_*` precedent (`limits` reserved for engine dimensions). No schema/example touched, so `scripts/validate.sh` is out of scope. Note: `idgen.rs:73` uses `unwrap_or_else(|_| unreachable!(...))` for a branch proven dead (36-char layout traced; version nibble `7`@14, variant@19, hyphens@8/13/18/23; `every_generated_id_is_a_valid_uuid_v7_...` exercises 1000 ids) — within the DoD's "asserted-impossible case" allowance and the idiomatic way to satisfy the denied `unwrap_used` lint; not a defect.

- **O4 — Reviewable: construct the fake bundle in a throwaway test, run it twice, and confirm byte-identical event streams and ids.**
  - *Claim:* a reviewer can construct the fake bundle in a throwaway test, run it twice, and observe byte-identical event streams and ids.
  - *Evidence to collect:* write and run a throwaway test that builds the fake bundle, drives it twice through the same sequence, and diffs the `RecordingEventSink` event stream and the `SeededIdGenerator` ids across the two runs — expect byte-identical output.
  - *Status:* ☑ SATISFIED — `two_fresh_bundles_drive_byte_identical_event_streams_and_ids` (lib.rs:620-641) builds `Fakes::new()`, drives the same `drive(...)` sequence twice over two fresh bundles, and asserts `stream_a == stream_b` (ndjson of the `RecordingEventSink`), `ids_a == ids_b` (the `SeededIdGenerator` ids), and `!stream_a.is_empty()`. Ran the test explicitly in isolation — PASS.

## Regression check

- Task 06 implements the Task 05 driven port traits. Trace that each fake's method signatures still match their Task 05 trait definition (`SerialScheduler`→`Scheduler`, `RecordingEventSink`→`EventSink`, `FixedClock`→`Clock`, `SeededIdGenerator`→`IdGenerator`, …) and that Task 05's traits still compile with the core I/O-free : ☑ PRESERVED — every `impl` compiles against the exact `tmx_core::ports::driven` signature (Rust rejects a mismatched impl; whole workspace builds and clippy is clean). Diff is scoped to `crates/tmx-testkit/**` + `Cargo.lock` + plan bookkeeping — `tmx-core`/`tmx-schema` sources untouched, purity gate still green, and no crate yet consumes `tmx-testkit`, so there is no downstream regression surface. The runner/conformance consumer is not yet built.

## Residue

- Task 06 step 3 says `RecordingEventSink` "asserts it routed every payload through the Masker," but the Masker is Task 09 — not yet built at Task 06. Treat the Masker-routing assertion as a forward reference (stubbed or deferred); do not fail O1/O2 on its absence, but note it as carried-forward work.
- The fake `SecretResolver`/`RunStore`/`SourceLoader` are provided "as the suite needs," so their presence is scoped to current consumers — a missing fake for a port with no consumer yet is acceptable if the bundle still type-checks for injected use cases.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☑ DONE — O1…O4 all SATISFIED, regression PRESERVED.
CONFIDENCE: ☑ high
SUMMARY: All 15 driven ports have a deterministic fake targeting its exact `tmx-core` trait, assembled into the injectable `Fakes` bundle; the determinism seam (serial index-ordered scheduler, frozen clock, seeded UUIDv7 ids) is proven with a live negative-space injection and a two-run byte-identity reviewable. cargo fmt/clippy/nextest (46/46) and scripts/purity.sh all pass; diff is scoped to tmx-testkit with no regression surface. The Masker-routing assertion on `RecordingEventSink` is correctly deferred to task 09 (Masker not yet built) per the certificate residue — carried-forward, not a gap.
