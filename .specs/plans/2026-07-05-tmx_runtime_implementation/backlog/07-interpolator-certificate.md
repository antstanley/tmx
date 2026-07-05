# Done Certificate — Task 07: Interpolator (the sandboxed `${{ }}` evaluator)

**Task:** [07-interpolator.md](07-interpolator.md) · **Plan:** [plan.md](../plan.md)
**State:** Authored 2026-07-05 — unverified

> This certificate is a verification protocol for Task 07. A validating agent discharges it:
> for each obligation, collect the named evidence, run the named checks, set the Status, then
> derive the Conclusion by the rubric below. Do not mark an obligation SATISFIED without its
> evidence; do not record DONE with any non-SATISFIED obligation.

## Definition

DONE(Task 07) ≡ every obligation O1…O4 below holds, each backed by the evidence the obligation
names (a file location, a test result, or an execution trace) — not by assertion. O4 is the
Reviewable item; record DONE only when O1…O4 are all SATISFIED.

## Premises

- **P1 — Goal.** Produce a pure `(expression, scope) -> Result<Value, RunError>` evaluator over a
  bounded, hand-written AST — the JavaScript subset with no engine and no `eval`.
- **P2 — Obligations.** Done iff O1…O4 all hold; O4 is the Reviewable item.
- **P3 — Invariants.** This is a new pure-core unit. It builds on the Task 04 `Scope`/model types in
  `crates/tmx-core/src/model.rs` and the Task 02 limit constants in `tmx-schema::limits`
  (`EXPR_LEN_MAX_BYTES`, `EXPR_DEPTH_MAX`); those must keep compiling and passing. No task consumes
  the evaluator yet — Task 11 will — so there is no in-scope caller to regress.

## Obligations

- **O1 — Every namespace resolves to its documented value; strict equality distinguishes `1` from `"1"`; `item.index`/`case`/`output` bind only in their construct.**
  - *Claim:* each of `inputs`/`env`/`secrets`/`tasks`/`item`/`case`/`output`/`matrix` resolves to the
    value [04 §State & interpolation scopes](../../../04-execution-engine.md) documents; `1 === "1"`
    is `false` (number vs string) while `1 === 1` is `true`; and `item`/`item.index`, `case`, and
    `output` are bound only inside a `map` inner task and `eval` scorer/subject respectively.
  - *Evidence to collect:* read the evaluator and `Scope` in `crates/tmx-core/src/interpolate.rs` and
    `crates/tmx-core/src/model.rs`; run `cargo nextest run -p tmx-core interpolate` and confirm the
    per-namespace resolution tests pass, a strict-equality test asserting `1 === "1"` is `false`
    passes, and a test that `item`/`case`/`output` are unbound (a `ResolutionError`, not a value)
    outside their construct passes.
  - *Checks:* trace resolution of `secrets.NAME` and confirm the listed-names guard (`task.secrets`)
    is consulted before the value is read; trace `1 === "1"` and confirm strict `===` dispatches to
    the type-and-value identity branch that separates a JSON number from a JSON string, not a
    coercing comparison.
  - *Status:* ☐ unverified

- **O2 — An over-length expression, an over-deep expression, an unknown namespace key, and an unlisted-secret reference each return the specified typed error rather than panicking, with boundary tests one below / at / one above each limit.**
  - *Claim:* an expression over `EXPR_LEN_MAX_BYTES` returns `ResolutionError` `expr_too_long`; an
    AST deeper than `EXPR_DEPTH_MAX` returns `ResolutionError` `expr_too_deep` (asserted at parse); an
    unknown namespace key returns `ResolutionError` (`unknown_namespace` / type mismatch); a reference
    to a secret not in `task.secrets` returns a `ResolutionError`; none of these panics.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core interpolate` and confirm the boundary
    tests exist and pass at one-below / at / one-above `EXPR_LEN_MAX_BYTES` and `EXPR_DEPTH_MAX`
    (below returns `Ok`, at/above returns the named error), plus an unknown-key test and an
    unlisted-secret test each asserting the typed error; run the property test
    (`interpolate` proptest module) that feeds malformed input and asserts no panic.
  - *Checks:* trace the length guard and confirm it short-circuits to `expr_too_long` before
    tokenizing; trace the depth guard and confirm the parser asserts `depth <= EXPR_DEPTH_MAX` and
    returns `expr_too_deep` rather than recursing past the bound; confirm the four error paths carry
    distinct codes and no `unwrap()`/`panic!` on the input path.
  - *Status:* ☐ unverified

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D
    warnings`, and `cargo fmt --all --check` — expect all clean; confirm the length and depth bounds
    are the named `tmx-schema::limits` constants `EXPR_LEN_MAX_BYTES`/`EXPR_DEPTH_MAX`, not magic
    numbers; run the `cargo tree` purity check (e.g. `cargo tree -p tmx-core -i tokio` expecting no
    match) confirming `tmx-core` takes on no async-runtime/I/O edge.
  - *Status:* ☐ unverified

- **O4 — Reviewable: run the interpolation unit + property tests, including the limit-boundary and unlisted-secret cases (Reviewable).**
  - *Claim:* a reviewer can run the interpolation test suite and observe the unit, property,
    limit-boundary, and unlisted-secret cases pass.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core interpolate` and read the summary;
    confirm the run includes the property tests and the boundary/unlisted-secret cases from O2 and
    reports zero failures.
  - *Status:* ☐ unverified

## Regression check

- No existing callers in scope — greenfield; nothing to regress. The Task 04 model and Task 02 limit
  constants this unit imports must still compile and pass.

## Residue

- DoD item 1 names `item`/`case`/`output` as scope-gated but not `matrix`; the validator should
  confirm `matrix.*` is also bound only under `--matrix` sugar (per 04's namespace table).
- JS truthy/falsy coercion (empty string, `0`, `null`, empty array/object) is a Step but folded into
  item 1's "documented value"; confirm the truthiness table matches JS semantics on the falsy cases.
- Confirm `EXPR_DEPTH_MAX` is enforced at parse (as the Steps require) and not only measured after a
  full parse, so a pathologically deep input cannot overflow the parser stack before the check.

## Conclusion

<!-- Validator derives this from the obligation statuses and the regression check, per the rubric. -->
VERDICT: ☐ (DONE | PARTIAL | NOT_DONE)
CONFIDENCE: ☐ (high | medium | low)
SUMMARY: ☐
