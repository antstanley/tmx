# Done Certificate — Task 35: Fan-out binding fidelity

**Task:** [35-fanout_binding_fidelity.md](35-fanout_binding_fidelity.md) · **Plan:** [plan.md](../plan.md)
**State:** Validated 2026-07-07

> Discharge each obligation with run/observed evidence; do not record DONE with any non-SATISFIED obligation.

## Obligations

- **O1 — `as:` alias honoured.** A `map` declaring `as: <name>` binds the element under `<name>`; `${{ <name>.* }}` resolves through the interpolator for object, array, and scalar elements. With no `as:`, the element binds under `item` (default preserved).
  - *Evidence:* tests + a real `tmx run` where `as: region` reads `${{ region }}`; the no-alias default reads `${{ item }}`.
  - *Status:* ☑ SATISFIED — `resolve_path` now resolves the dynamic root via `_ if root == item_alias` (default `DEFAULT_ITEM_ALIAS = "item"`); unit tests `the_as_alias_binds_the_element_under_its_name`, `the_alias_replaces_item_so_bare_item_is_unbound` and e2e `a_map_with_an_as_alias_binds_the_element_and_its_index_end_to_end` pass. Real binary: `as: region` over `["us-east","eu-west"]` produced `us-east at 0` / `eu-west at 1`; no-alias produced `a hash 0` / `b hash 1`.
- **O2 — `.index` unconditional.** The synthetic `.index` is injected for every element type; `${{ item.index }}` / `${{ <alias>.index }}` yields the position for scalar and array elements, not just objects.
  - *Evidence:* a scalar-element map test observes `.index`; the element value stays readable.
  - *Status:* ☑ SATISFIED — `.index` is synthesised from `scope.item_index` for non-object elements via a surgical guard (lone `.index` access only); objects keep their merged/own key (`an_object_element_with_its_own_index_wins_over_the_synthetic_one`). Tests `the_synthetic_index_resolves_for_scalar_and_array_elements`, `a_scalar_element_map_reads_item_and_its_synthetic_index_without_an_alias` pass; real binary `${{ item.index }}` yielded `0`/`1` for scalar elements. Negative: `a_scalar_field_other_than_index_is_still_a_type_mismatch`.
- **O3 — no regression.** Unknown namespaces still error typed; existing map/eval tests stay green; `cargo fmt --all --check` / `clippy -all-targets --all-features -D warnings` / `nextest` (all prior + new) / `scripts/purity.sh` all clean.
  - *Status:* ☑ SATISFIED — ran from repo root: `cargo fmt --all --check` clean (exit 0); `cargo clippy --all-targets --all-features -- -D warnings` clean (exit 0); `cargo nextest run` 468 passed / 0 failed; `scripts/purity.sh` green. Aliased-away `item` still errors typed `unknown_namespace` (unit + e2e `an_aliased_map_leaves_the_literal_item_root_unbound`).
- **O4 — Reviewable** exercised on the real binary per the task's Reviewable line.
  - *Status:* ☑ SATISFIED — `tmx run` on a scalar-array map with `as: region` printed `us-east at 0` / `eu-west at 1` (`${{ region }}`/`${{ region.index }}`), and the same flow without `as:` printed `a hash 0` / `b hash 1` (`${{ item }}`/`${{ item.index }}`).

## Conclusion
VERDICT: DONE
CONFIDENCE: high
SUMMARY: The `as:` alias is resolved via a dynamic root in `resolve_path` (default `item` preserved), and `.index` is threaded through `Scope::item_index` so scalar/array elements resolve their position — with object-own-index precedence and non-index scalar fields still type_mismatch preserved. All four obligations SATISFIED: fmt/clippy/nextest (468 pass)/purity clean, and the real `tmx` binary printed `us-east at 0`/`eu-west at 1` (aliased) and `a hash 0`/`b hash 1` (default `item`).
