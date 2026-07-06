# Task 35 — Fan-out binding fidelity (`as:` alias + `.index`)

**Plan:** [plan.md](../plan.md) · **Certificate:** [35-fanout_binding_fidelity-certificate.md](35-fanout_binding_fidelity-certificate.md)

**Implements:** [05-fan-out-and-eval.md](../../../05-fan-out-and-eval.md) §The `map` task (element binding), §Required read patterns; [04-execution-engine.md](../../../04-execution-engine.md) §Interpolation namespaces
**Depends on:** —
**Produces:** a `map` task's declared `as:` alias binds the current element (so `${{ <alias>.* }}` resolves), and every element binding carries a synthetic `.index` regardless of element type (object, array, or scalar) — both exercised end-to-end.
**Pointers:** `crates/tmx-core/src/fanout.rs` (`run_map` → `bind_item`; currently binds unconditionally under `item` and injects `.index` only for `Value::Object`), `crates/tmx-core/src/interpolate.rs:583-606` (`resolve_path` hardcodes the root namespace `item`; a non-`item` root falls through to a typed `unknown_namespace` error), `crates/tmx-schema/src/task.rs:327` (`MapWith.as_binding`, schema field `as`).

## Steps

- [x] Honour `MapWith.as_binding`: when a `map` declares `as: <name>`, bind the current element under `<name>` (in addition to or instead of `item`, per the spec's default-`item` rule) so `${{ <name>.* }}` resolves through the interpolator's `Scope`. Thread the alias from `run_map`/`bind_item` into the `Scope` the inner task interpolates against; the interpolator must resolve the dynamic root, not just the literal `item`.
- [x] Inject the synthetic `.index` for **every** element type (object, array, scalar), not only `Value::Object` — the spec says `.index` is unconditional. For a scalar/array element, `${{ item.index }}` (or `${{ <alias>.index }}`) yields the element's position; the element's own value stays readable.
- [x] Preserve the default: with no `as:`, the element still binds under `item` exactly as today.
- [x] Add tests: a `map` with `as: region` reads `${{ region }}` / `${{ region.index }}`; a scalar-element map reads `${{ item.index }}`; the no-`as:` default still binds `item`; unknown namespaces still error.

## Definition of done

- [x] A `map` task's `as: <name>` alias resolves via `${{ <name>.* }}` for object, array, and scalar elements; with no `as:` the element binds under `item`.
- [x] `.index` is available for every element type (not just objects).
- [x] Meets the repo definition of done (tests incl. negative space, `cargo fmt`/`clippy -D warnings`/`nextest`/`scripts/purity.sh` clean).
- [x] Reviewable: `tmx run` a `map` flow declaring `as: region` over a scalar array and observe each `${{ region }}`/`${{ region.index }}` in the collected output; run the same without `as:` and observe `${{ item }}` still works.
