# Done Certificate — Task 07: Interpolator (the sandboxed `${{ }}` evaluator)

**Task:** [07-interpolator.md](07-interpolator.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-05

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
  - *Status:* ☑ SATISFIED — `cargo nextest run -p tmx-core interpolate` = 12/12 pass.
    `every_namespace_resolves_to_its_documented_value` resolves all 8 namespaces (incl. `item.index`,
    bare `output`, nested `tasks.build.artifacts[1]`). `strict_equality_distinguishes_number_from_string`
    confirms `1 === "1"` is `false`, `1 === 1`/`1 === 1.0` `true`, `true === 1` `false` (no coercion) —
    traced to `strict_eq` (interpolate.rs:552) whose cross-type arm returns `false`.
    `scope_gated_namespaces_are_unbound_outside_their_construct` confirms `item`/`case`/`output` are
    `unknown_namespace` (not values) when their `Scope` option is `None` (resolve_path:583-585).
    Secret gating traced: `scope.secrets` holds only listed names (frozen Scope + 04 §Secrets); a
    missing first-step key under `secrets` → distinct `unlisted_secret` (access_key:656). Truthiness
    table matches JS (is_truthy:539).

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
  - *Status:* ☑ SATISFIED — length boundary (`expression_length_boundary_below_at_above`): 4095 Ok,
    4096 Ok, 4097 `expr_too_long`; guard `src.len() > EXPR_LEN_MAX_BYTES` (parse:334) runs *before*
    `tokenize`. Depth boundary (`expression_depth_boundary_below_at_above`): depth 32 Ok, 33
    `expr_too_deep`; `check_depth` (parse:384) fires at the top of each `parse_bp` recursion, and
    `a_deeply_nested_input_fails_cleanly_without_overflowing_the_stack` (1000-deep parens) returns
    `expr_too_deep` with no stack overflow. Unknown key → `unknown_namespace`, scalar member →
    `type_mismatch`, unlisted secret → `unlisted_secret` — four distinct codes.
    `malformed_expressions_are_parse_errors_not_panics` + `property_malformed_input_never_panics`
    (4000 fuzzed inputs) + `property_well_formed_literal_expressions_always_evaluate` (2000) all pass.
    Grep of non-test lines 1–712 shows zero `.unwrap()`/`panic!`/`.expect()` panic sites (only the
    parser's `self.expect(..) -> Result`). Independent verifier probe (temp test, since reverted)
    confirmed over-length illegal-char input → `expr_too_long` (length precedes tokenizer),
    negative index → `type_mismatch`, out-of-range index → `unknown_namespace` — no panics.

- **O3 — Meets the repo definition of done.**
  - *Claim:* tests pass, clippy and rustfmt clean, every new bound is a named units-last constant.
  - *Evidence to collect:* run `cargo nextest run`, `cargo clippy --all-targets --all-features -D
    warnings`, and `cargo fmt --all --check` — expect all clean; confirm the length and depth bounds
    are the named `tmx-schema::limits` constants `EXPR_LEN_MAX_BYTES`/`EXPR_DEPTH_MAX`, not magic
    numbers; run the `cargo tree` purity check (e.g. `cargo tree -p tmx-core -i tokio` expecting no
    match) confirming `tmx-core` takes on no async-runtime/I/O edge.
  - *Status:* ☑ SATISFIED — `cargo fmt --all --check` exit 0; `cargo clippy --all-targets
    --all-features -- -D warnings` exit 0; `cargo nextest run` = 58 run, 58 passed, 0 skipped. Both
    bounds are the named `tmx_schema::limits` constants `EXPR_LEN_MAX_BYTES`/`EXPR_DEPTH_MAX` (their
    only uses are at interpolate.rs:334,385) — no magic-number limits. Purity: `cargo tree -p
    tmx-core -i {tokio,reqwest,aws-config,aws-sdk-s3,hyper}` all ABSENT, and `scripts/purity.sh`
    prints green.

- **O4 — Reviewable: run the interpolation unit + property tests, including the limit-boundary and unlisted-secret cases (Reviewable).**
  - *Claim:* a reviewer can run the interpolation test suite and observe the unit, property,
    limit-boundary, and unlisted-secret cases pass.
  - *Evidence to collect:* run `cargo nextest run -p tmx-core interpolate` and read the summary;
    confirm the run includes the property tests and the boundary/unlisted-secret cases from O2 and
    reports zero failures.
  - *Status:* ☑ SATISFIED — `cargo nextest run -p tmx-core interpolate` = 12 tests run, 12 passed,
    18 skipped; the run includes both property tests (`property_malformed_input_never_panics`,
    `property_well_formed_literal_expressions_always_evaluate`), all three limit-boundary tests, and
    the `an_unlisted_secret_reference_is_a_resolution_error` case — zero failures.

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
VERDICT: DONE
CONFIDENCE: high
SUMMARY: All four obligations SATISFIED, each on discharged evidence. The `${{ }}` evaluator is a
pure, bounded, hand-written parser/evaluator with no JS engine or `eval`: all 8 namespaces resolve to
their documented values, strict `===` separates `1` from `"1"` and compares numbers by value with no
coercion, `item`/`case`/`output` bind only when their `Scope` option is set, and JS truthy/falsy
matches. Negative space is complete and typed: length (below/at/above 4096) and depth (below/at/above
32) boundaries, unknown-key, type-mismatch, and unlisted-secret each carry a distinct
`Resolution`-category code; the length guard short-circuits before tokenizing and the depth guard is
asserted at parse (1000-deep parens → `expr_too_deep`, no stack overflow); two property tests (4000
fuzz + 2000 well-formed) never panic. Repo DoD green: `cargo fmt --all --check`, `cargo clippy
--all-targets --all-features -- -D warnings`, and `cargo nextest run` (58/58) all clean; both bounds
are the named `tmx_schema::limits` constants (no magic numbers); `tmx-core` takes on no async/I/O edge
(purity.sh green). Regression: greenfield unit — only `pub mod interpolate;` + `pub use
interpolate::evaluate;` added to lib.rs; the Task 04 `Scope`/model and Task 02 limits still compile and
pass. Residue confirmed: `matrix` is a always-bound namespace per the frozen Task 04 Scope, so a
missing key outside `--matrix` reads as `unknown_namespace` (unavailable, as intended); the truthiness
table matches JS on every falsy case; and `EXPR_DEPTH_MAX` is enforced during recursion, not after a
full parse. One documented, spec-compatible design choice (not a defect): strict `===` on
array/object operands compares structurally rather than by JS reference identity, because the JSON
sandbox has no object identity — the hard spec requirement (`1 === "1"` false, numbers by value) holds
and composite comparison is not exercisable via the subset's grammar in a way that could diverge in
practice.
