# Task 06 — Testkit fake adapters

**Plan:** [plan.md](../plan.md) · **Certificate:** [06-testkit_fakes-certificate.md](06-testkit_fakes-certificate.md)

**Implements:** [02-crate-architecture.md](../../../02-crate-architecture.md) §tmx-testkit; [architecture-principles.md](../../../architecture-principles.md) §2.5 Determinism and testability; [development-guidelines.md](../../../development-guidelines.md) §Testing (Determinism)
**Depends on:** 05
**Produces:** one in-memory fake per driven port plus a fake-bundle constructor, so the core, the conformance suite, and downstream embedders inject one shared deterministic port set
**Pointers:** `crates/tmx-testkit/src/lib.rs` (new), `crates/tmx-testkit/src/{scheduler,clock,idgen,process,http,chat,fs,store,sink}.rs` (new)

## Steps

- [ ] Implement the determinism seam: `SerialScheduler` (strictly serial, index-ordered, asserts `concurrency >= 1` and in-flight `<= concurrency`), `FixedClock` (frozen, step-advanceable), and `SeededIdGenerator` (deterministic UUIDv7 sequence).
- [ ] Implement the recording I/O fakes: `RecordingProcessRunner` (scripted stdout/exit), `FakeHttpClient` (canned responses), `FakeChatModel` (canned completions), `MemFileSystem`, `MemObjectStore`, and a fake `SecretResolver`/`RunStore`/loader as the suite needs.
- [ ] Implement `RecordingEventSink` that captures the event stream for assertion and asserts it routed every payload through the Masker.
- [ ] Provide a fake-bundle constructor assembling the full port set, and keep the crate dependent on `tmx-core` + `tmx-schema` only — no `tokio`, no `reqwest`, no I/O crate.

## Definition of done

- [ ] The fake bundle satisfies every driven port trait and is injectable into a use case; the `cargo tree` purity gate confirms `tmx-testkit` pulls in no async runtime or I/O crate.
- [ ] `SerialScheduler` returns results in index order for a shuffled completion order (negative space: a non-index-ordered result fails the test), and `FixedClock`/`SeededIdGenerator` produce identical sequences across two runs.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: construct the fake bundle in a throwaway test, run it twice, and confirm byte-identical event streams and ids.
