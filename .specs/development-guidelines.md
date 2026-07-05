# Development Guidelines

**Status:** Draft · **Date:** 2026-05-31 · **Owner:** Ant Stanley · **Scope:** Repo-wide

The rules of the road for everyone — humans and agents — writing code in the TMX Rust implementation.
[architecture-principles.md](architecture-principles.md) sets the *architectural* rules (hexagonal
layering, the Tiger Style tenets, the Rust conventions at the macro level); this page is the
*developer-level* discipline: toolchain, defensive coding, limits, version control, testing, and the
definition of done. The two are consistent by construction — this page is the day-to-day form of the
principles there.

The pillars: **adopt Tiger Style**, **validate at every boundary**, **bound everything with a named
constant**, **assert invariants in release**, **keep the core pure**, **leave zero technical debt**.

> The Rust toolchain described below is the **intended** toolchain for the implementation; no Rust
> code exists yet (the repo is a pre-runtime spec). The only enforcement wired today is the schema
> validation in [`scripts/validate.sh`](../../scripts/validate.sh) and the jj/git pre-push gate. The
> Rust gates (`fmt`/`clippy`/`nextest`) join the pre-push hook as the first crate lands — see
> [Open questions](#assumptions-and-open-questions).

---

## Toolchain

| Tool | Version / channel | Notes |
|---|---|---|
| Rust | stable, pinned; **MSRV 1.96.1** | pinned in `rust-toolchain.toml`; MSRV 1.96.1 declared in the workspace `Cargo.toml` (`rust-version`) and tested in CI |
| Edition | 2024 | workspace-wide |
| rustfmt | default channel | `max_width = 100`; runs in CI and pre-push |
| clippy | latest | `--all-targets --all-features -D warnings`; `clippy.toml` opt-outs each carry a comment |
| test runner | cargo-nextest | the runner CI runs; `cargo test` for doctests |
| schema validator | `scripts/validate.sh` (Python) | validates the data-model schemas + examples; the current pre-push gate until `tmx validate` reaches parity |
| version control | jujutsu (`jj`) | git backend present; jj is the front end ([Version control](#version-control-jujutsu)) |

---

## Tiger Style — the pervasive style

This project adopts **Tiger Style** as its pervasive coding style. It is the default, not a
recommendation; deviations require a written reason in the change description. The pragmatic form is
adopted (see [architecture-principles.md](architecture-principles.md#2-tiger-style-tenets-as-applied-here)):
the bounded/assertion/limit core in full, static allocation relaxed to a hard, asserted cap on the
dynamic Pipeline state.

The short form: **be defensive and validate everything.** Assume any input you did not produce is
wrong. Assume any invariant you did not assert can be violated. Make every limit explicit, every error
handled, every assumption checked. Design priorities are **safety, performance, developer experience —
in that order**; when they conflict, safety wins.

Load-bearing principles:

- **Zero technical debt.** Do it right the first time; no "TODO: fix later" in merged code. The task
  enum is closed and no plugin port is built until specified — unbuilt capability is not scaffolded.
- **Simple, explicit control flow.** No recursion except the one bounded, asserted case (`flow`
  import; see [Code style](#code-style)). No clever combinators that hide a non-trivial branch.
- **Limits on everything.** Every loop, fan-out, recursion, payload, and buffer has an explicit named
  bound. The full set is the [limits table in 04-execution-engine.md](04-execution-engine.md#limits).
- **Assertions are first-class code.** They detect programmer errors; the only correct response to a
  violated assertion is to abort. Average **at least two assertions per function** in the core.
- **Always say *why*.** Comments and change descriptions explain the rationale, not the action.

---

## Defensive coding and assertions

### Where to validate

Validate at every boundary where data crosses from a place you do not control into one you do.

| Boundary | What to validate | How |
|---|---|---|
| Source file → loader | Format well-formedness, then schema | `SourceLoader` parse, then `SchemaValidator` (Draft 2020-12), `kind`-dispatched |
| Preflight → runner | Domain invariants, limits | Validate every artifact; reject over-limit task counts / fan-out widths / depths up front |
| Core function entry | Preconditions | `assert!` arguments at the top of every core function |
| Adapter → core | Adapter contract | Assert the shape of what a driven port returns (state is an object; scores ∈ [0,1]) |
| `--state-in` read | Round-trip integrity | Re-validate seeded state on read, even state TMX wrote |
| Third-party API (`fetch`/`store`/`chat-completion`) | Status, content type, body shape, size | Treat as adversarial; never deserialise and trust; bound captured output |

Malformed **input** is caught by the validator and returned as a typed `ValidationError` — it is never
asserted. Assertions are for **programmer** errors (broken invariants), not for user data.

### Assertions in Rust

- Use `assert!`, `assert_eq!`, and `debug_assert!` liberally in core code. **Production builds keep
  `assert!` enabled** — no `--release`-only assertions for invariants whose violation means corrupt
  state or a leaked secret.
- **Average two or more assertions per core function**: preconditions on entry, postconditions on
  exit, invariants in the middle. `assert!(true)` does not count. The runner's load-bearing set is the
  [Invariants section](04-execution-engine.md#invariants--assertions).
- **Pair assertions.** Enforce a property along two independent paths: the Masker registers every
  secret *before* any output port runs, and each output port asserts it routed through the Masker;
  `map` asserts output length equals input length on both producing and consuming sides.
- **Assert positive and negative space** — what you expect, and what must never happen (nothing leaves
  unmasked; no lifecycle hook fires inside a hook).
- **Compile-time assertions** for size/layout and limit sanity: `const _: () = assert!(FLOW_DEPTH_MAX
  >= 1);`.
- **Split compound assertions:** `assert!(a); assert!(b);` over `assert!(a && b)` — a failure points at
  the actual broken condition.
- **No `unwrap()` / `expect()` / `panic!` in non-test code**, except an asserted-impossible case — and
  that case is an `assert!`, not an `unwrap()`. A panic signals a programmer error only, never control
  flow or an expected host failure.

### Errors are data, not exceptions

- Every error is a value: the core returns [`RunError`](08-errors-and-observability.md#error-model)
  carrying a typed `ErrorCategory`, a stable `code`, and a message. `thiserror` derives the enums.
- **`anyhow` is not used in the core or adapters** — it erases the category the exit-code mapping
  depends on. The CLI may use it only at the outermost `main` seam.
- **Every error is handled or explicitly propagated.** Swallowing an error is a bug; `#[must_use]` and
  `unused_must_use` are denied.
- **Retry policies are explicit and bounded** — `fetch` retries have a named max; no unbounded retry.
- **Never log a secret.** Masking is structural at the output boundary, not a per-call courtesy.

### Make invalid states unrepresentable

- Use the type system. `RunId` is a newtype, not a bare `String`; `Duration` is normalised to
  `Milliseconds` at resolution. The `TaskWith` enum makes "a task with a `with` for the wrong type"
  unrepresentable past deserialisation.
- Model state with enums matched **exhaustively, no fallthrough** — `RunStatus`, `TaskStatus`,
  `ErrorCategory`, `MatcherName`. The dispatcher asserts exhaustiveness over the task enum.
- Order-preserving maps (`IndexMap`) for the task map form, so "runs in source key order" is a type
  property, not a convention to remember.

---

## Limits and bounds

Every limit is a **named constant** with its units, defined once in `tmx-schema::limits`, referenced
everywhere it applies. No magic numbers. Constants are named **units-last, descending significance** —
`STATE_SIZE_MAX_BYTES`, `FLOW_DEPTH_MAX`, `EXPR_LEN_MAX_BYTES` — not `MAX_STATE_SIZE`.

The *existence* of a limit is non-negotiable (this is the global rule); the concrete *values* are this
implementation's concern and live in the [limits table in 04-execution-engine.md](04-execution-engine.md#limits),
not here. Reaching a limit is an **observable event**: it emits a typed error naming the limit and (for
runtime caps) a diagnostic — it never silently truncates or drops. A new loop, fan-out, buffer, or
recursion ships with its named bound in the same change.

---

## Version control: jujutsu

This repo is managed with **Jujutsu (`jj`)** over a Git backend. jj is the front end.

### Shared core

- **Commits are small and well-described.** One coherent change per commit; squash noise before
  pushing. An empty description is not accepted — describe the *why*.
- **Conventional Commits** for the subject: `type(scope): subject` (`feat`, `fix`, `docs`, `chore`,
  `refactor`, `test`, `build`, `ci`, `perf`, `style`). The repo's history already follows this.
- **`main` stays releasable.** Feature work happens on named bookmarks; changes are pushed for review.
- **Destructive operations need explicit confirmation** — history rewrites, bookmark deletion,
  `jj abandon`, `jj op restore` — even when they look like the cleanest path.

### jj specifics

- **`jj` is the sole front end.** Do not run `git commit` / `git add` / `git status` against the jj
  working copy — the index/working-copy mismatch is exactly what jj removes.
- **jj does not run Git hooks.** Push with [`scripts/push.sh`](../../scripts/push.sh), which runs the
  validation gate and then `jj git push`. A plain `git push` (colocated repo / CI) is backstopped by
  [`.githooks/pre-push`](../../.githooks/pre-push), enabled once with `git config core.hooksPath
  .githooks`.
- **Describe before pushing** (`jj describe`); **feature work on named bookmarks**
  (`jj bookmark create feat/x`); **resolve conflicts in jj** (`jj resolve`), not by editing markers.
- The `.jj/` directory is local and not committed.

The pre-push gate runs the schema validation today; as crates land it also runs `cargo fmt --check`,
`cargo clippy -D warnings`, and the fast `nextest` tier. CI re-runs the same plus the slow tier.

---

## Rust conventions

### Formatting and linting

- `cargo fmt --all` clean before every push (`max_width = 100`).
- `cargo clippy --all-targets --all-features -D warnings` clean before every push.
- `clippy.toml` enables pedantic-adjacent lints; each opt-out carries a comment explaining why.
  `clippy::unwrap_used` and `clippy::expect_used` are **denied outside tests**;
  `#![forbid(unsafe_code)]` is in every crate.

### Code style

- **Modules over files.** Many small files; the [module trees in 02](02-crate-architecture.md#crates)
  are the target granularity. No business logic in `main.rs` or in command modules — they parse,
  compose, call a use case, serialise.
- **Hard limit: 70 lines per function.** Longer means split — extract pure helpers, centralise control
  flow in the parent ("push `if`s up, push `for`s down").
- **Hard limit: 100 columns per line**, via rustfmt.
- **No recursion, except the one bounded case.** The `flow`-import recursion into `PipelineRunner` is
  the rare unavoidable case; it **asserts its bound (`FLOW_DEPTH_MAX`) at entry** (see
  [04](04-execution-engine.md#bounded-flow-recursion)). Everything else iterates with an explicit
  upper bound.
- **Explicit fixed-width integers** (`u32`, `u64`) for domain values; avoid `usize` across a
  serialisation boundary.
- **The core never sees a third-party error type** — `From`/`TryFrom` impls translate every vendor
  error (`reqwest`, the S3 SDK) into a `RunError` at the adapter boundary.
- **Simpler return types win:** `()` > `bool` > integer > `Option<T>` > `Result<T, E>`. Chains of
  `.map().and_then().ok_or()` that hide branches are a smell; prefer an explicit `match` when control
  flow is non-trivial.
- **Pass large structs by reference** (`> 16` bytes and not moved → `&T`).
- **State invariants positively:** `if depth < FLOW_DEPTH_MAX` over `if depth >= …` when expressing
  the holding case.
- **`#[must_use]`** on `Result`, on port handles, and on builders.
- **Comments explain *why*** in full sentences — an invariant, a workaround, a non-obvious constraint
  a future reader would miss. No comment that paraphrases the code.

### Naming

- `snake_case` for functions, variables, modules, files; `CamelCase` for types and traits (Rust
  tooling depends on it). **Acronyms in proper case**: `HttpClient`, not `HTTPClient`.
- **No abbreviations** beyond ecosystem-standard short names (`ctx`, `cfg`, `id`, loop counters).
- **Units last in identifiers, descending significance:** `state_size_max_bytes`, `latency_ms`;
  limit constants follow the same rule (`STATE_SIZE_MAX_BYTES`).
- **Same-length names for related variables** where reasonable: `source` / `target`, not `src`/`dst`.
- **Helpers prefix with the parent name** to show call history; callbacks go last in parameter lists.

### Testing

- **cargo-nextest** is the sanctioned runner; `cargo test` covers doctests.
- **Test pyramid.** In-module unit tests for the pure core (`Interpolator`, `Masker`, `MatcherEngine`,
  merge); integration tests per crate exercising the core plus in-memory adapters; end-to-end
  **golden Flows** that drive `RunFlow` with recorded adapters and assert the event stream + final
  state (marked `#[ignore]` when they need real backends).
- **Determinism.** Inject the `Clock`, `IdGenerator`, and `SerialScheduler` fakes from `tmx-testkit`;
  no `SystemTime::now()`, no randomness, no `TokioScheduler` in test bodies — this is the determinism
  payoff of the hexagon ([architecture-principles.md](architecture-principles.md#25-determinism-and-testability)).
- **Positive and negative space together.** Every validation path ships with a test for what it
  accepts *and* what it rejects (a leaked-secret test, an over-cap-state test, a too-deep-recursion
  test).
- **Test the validity boundary** — one below a limit, at the limit, one above.
- **Property tests** (`proptest`) for the interpolation parser, the matcher engine, and state merge.
- **No flaky tests.** A flaky test is a bug fixed immediately, not retried.

### Documentation

- Public items in `tmx-core`/`tmx-schema` carry doc comments. Each crate's `lib.rs` documents what the
  crate is, the ports it depends on, and the surface it offers.
- No bare `// TODO` without an owner and a tracking reference.

---

## Repository hygiene

- **`docs/`** is the canonical home for specs (this set) and the design drafts. Code lives in
  `crates/`.
- **Operator data is gitignored.** The run store at `./.tmx/runs/` is local, untracked; never commit
  it, environment-specific config, or secrets.
- **The pre-push gate** runs format-check, lint, and the fast test tier (plus the schema validation
  that exists today). CI re-runs the same plus the slow tier and the golden Flows.
- **Generated artifacts are checked in** and grep-able; a CI job regenerates and fails on drift. The
  example corpus parity check ([`scripts/validate_examples.py`](../../scripts/validate_examples.py))
  is the existing instance.

---

## Guidelines for AI agents

Not different rules — emphasis on where agents slip.

1. **The pervasive style applies to you too.** Defensive validation and explicit limits are not
   optional, even on a small change.
2. **Add assertions as you go.** Every core function you touch leaves with at least two meaningful
   assertions. Asserting a constant truth does not count.
3. **No silent error swallowing.** Every error is handled; every match on `RunStatus`/`TaskStatus`/
   `ErrorCategory`/`MatcherName`/`TaskWith` is exhaustive.
4. **Stay inside the architecture.** Adding I/O directly to `tmx-core` is the most common slip —
   define a port, implement an adapter in `tmx-adapters`, call into it. The dependency rule
   (`cli → adapters → core`) is enforced by the crate graph; do not subvert it.
5. **Do not add backwards-compat shims.** If a type changes, change every caller. There is no
   published API to preserve.
6. **Do not invent runtime fields.** New output shapes go in
   [`canonical-types.schema.json`](canonical-types.schema.json) first; new input fields belong to the
   data-model schema, which this implementation does not change unilaterally.
7. **Tests run before claiming complete.** "Compiles" is not "works". Run `nextest` and report the
   actual output.
8. **Test positive and negative space together.** A new feature ships with tests for what it accepts
   *and* what it rejects.
9. **Limits are explicit.** A new loop, fan-out, retry, or buffer ships with a named units-last
   constant in `tmx-schema::limits` in the same change.
10. **Prefer small, frequent commits** with described *why*.
11. **No comments that paraphrase the code.**
12. **Do not run destructive `jj`/git operations without explicit confirmation**, and **do not skip
    the pre-push gate** (`--no-verify` / bypassing `scripts/push.sh`). If it fails, fix the cause.

---

## Definition of done

A change is done when:

- The behaviour is exercised by a test (unit, integration, or a golden Flow as appropriate).
- The change includes **negative-space tests** for every new validation path.
- Every new or touched core function has at least two meaningful assertions.
- Every new bound is a named units-last constant in `tmx-schema::limits`.
- `cargo fmt --all`, `cargo clippy --all-targets --all-features -D warnings`, and `cargo nextest run`
  pass locally; `scripts/validate.sh` passes if any schema/example changed.
- If runtime output shapes changed, [`canonical-types.schema.json`](canonical-types.schema.json) is
  updated; if the data model changed, the change is coordinated with the data-model schema and
  `CHANGELOG.md`.
- The change description states the *why* and lists what changed at the architecture level.

---

## Assumptions and open questions

**Assumptions**

- cargo-nextest is available in CI and locally; `cargo test` remains for doctests.
- A `clippy.toml` and a `rustfmt.toml` (or `[lints]` in the workspace `Cargo.toml`) can express the
  denied lints and the 100-column width.

**Decisions**

- *Tiger Style, pragmatic form.* **The bounded/assertion/limit discipline in full; static allocation
  relaxed to a hard asserted cap.** Chosen because TMX's identity is dynamic JSON dataflow over async
  backends; the safety properties that matter (no unbounded growth, no leaked secret, no swallowed
  error) are kept. See [architecture-principles.md](architecture-principles.md#2-tiger-style-tenets-as-applied-here).
- *jj-first version control.* **Push via `scripts/push.sh`; `.githooks/pre-push` backstops plain
  `git push`.** Chosen because jj does not run Git hooks, so the validation gate must be invoked by
  the jj push wrapper.
- *Assertions on in release.* **`assert!` for invariants, not only `debug_assert!`.** Chosen so a
  corrupt-state or leaked-secret bug aborts in production rather than miscomputing.
- *Limit constants are units-last.* **`STATE_SIZE_MAX_BYTES`, not `MAX_STATE_SIZE`.** Chosen for Tiger
  Style naming fidelity (units last, descending significance).
- *No numeric coverage gate.* **CI has no coverage-percentage floor; the golden-Flow conformance
  suite plus the negative-space test rules are the correctness net.** Chosen because a percentage
  gate invites gaming and measures lines, not properties; revisit only if regressions slip
  through.
- *MSRV pinned at 1.96.1.* **The minimum supported Rust version is 1.96.1, declared via `rust-version`
  in the workspace `Cargo.toml`, pinned in `rust-toolchain.toml`, and verified in CI.** Raising the
  MSRV is a deliberate, `CHANGELOG.md`-noted change, not an automatic follow of stable. Chosen as a
  recent stable that comfortably clears the edition 2024 floor (Rust 1.85), giving the current language
  and `std` surface while keeping one explicit floor every crate builds against.

**Open questions**

- *Rust toolchain not yet wired.* No crate, `Cargo.toml`, `rust-toolchain.toml`, `clippy.toml`, or
  `nextest` config exists — the repo is a pre-runtime spec. The toolchain rows and the fmt/clippy/test
  pre-push gates are the *intended* setup; they become body-true (and the
  [`scripts/validate.sh`](../../scripts/validate.sh) gate extends to call them) as the first crate
  lands. Until then they are the plan, recorded here.
