# Task 07 — Interpolator (the sandboxed `${{ }}` evaluator)

**Plan:** [plan.md](../plan.md) · **Certificate:** [07-interpolator-certificate.md](07-interpolator-certificate.md)

**Implements:** [04-execution-engine.md](../../../04-execution-engine.md) §State & interpolation scopes; [01-domain-model.md](../../../01-domain-model.md) §Required read patterns (interpolation namespaces)
**Depends on:** 04
**Produces:** a pure `(expression, scope) -> Result<Value, RunError>` evaluator over a bounded, hand-written AST — the JavaScript subset with no engine and no `eval`
**Pointers:** `crates/tmx-core/src/interpolate.rs` (new), `crates/tmx-core/src/model.rs` (the `Scope` struct)

## Steps

- [ ] Implement the `Scope` as borrowed references exposing the `inputs`/`env`/`secrets`/`tasks`/`item`/`case`/`output`/`matrix` namespaces, with `secrets` visible only for names the task listed.
- [ ] Implement the tokenizer and a recursive-descent parser for the subset: member access, literals, strict `===`/`!==`, boolean/`!` logic, and JS truthy/falsy — no function calls, no assignment, no arbitrary code.
- [ ] Bound the parser: reject an expression over `EXPR_LEN_MAX_BYTES` (`expr_too_long`) and an AST deeper than `EXPR_DEPTH_MAX` (`expr_too_deep`) as `ResolutionError`s; assert the AST depth bound at parse.
- [ ] Evaluate against the scope, returning `ResolutionError` (`unknown_namespace` / type mismatch) for an unknown key or a bad input coercion, and referencing an unlisted secret as a resolution failure.
- [ ] Add property tests over the parser and evaluator (well-formed expressions round-trip; malformed input never panics).

## Definition of done

- [ ] Each namespace resolves to the documented value, strict equality distinguishes `1` from `"1"`, and `item.index`/`case`/`output` bind only in their construct.
- [ ] An over-length expression, an over-deep expression, an unknown namespace key, and an unlisted-secret reference each return the specified typed error rather than panicking (negative space), covered by boundary tests one below / at / one above each limit.
- [ ] Meets the repo definition of done (tests, lint/format, named-constant limits — see plan.md baseline).
- [ ] Reviewable: run the interpolation unit + property tests, including the limit-boundary and unlisted-secret cases.
