#!/usr/bin/env bash
# The full TMX pre-push gate: everything that must be green before a push. Shared by
# scripts/push.sh (the jj front end) and .githooks/pre-push (the plain `git push` backstop) so both
# enforce the identical set; CI re-runs the same (development-guidelines.md §Version control).
#
# Order is cheapest-signal-first: format, then lint, then tests, then the dependency-purity gate,
# then the schema/example validation that already existed. Any failure aborts with a non-zero exit.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "gate: cargo fmt --all --check"
cargo fmt --all --check

echo "gate: cargo clippy --all-targets --all-features -D warnings"
cargo clippy --all-targets --all-features -- -D warnings

# `cargo nextest run` spans the whole workspace, which now includes the `tmx-conformance` crate —
# the slow conformance tier (golden Flows over the recorded fakes, the limit-boundary trios, the
# negative-space cases, and the property-test tier). It is a workspace member, so the gate/CI run it
# here with no extra step; none of its golden Flows is `#[ignore]`d, since every one is driven over
# the deterministic testkit fakes rather than a real backend.
echo "gate: cargo nextest run (incl. the tmx-conformance slow tier)"
cargo nextest run

echo "gate: dependency-purity check"
"$ROOT/scripts/purity.sh"

echo "gate: schema + example validation"
"$ROOT/scripts/validate.sh"

echo "✓ gate passed (fmt · clippy · nextest · purity · schema)."
