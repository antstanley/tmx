#!/usr/bin/env bash
# Negative-space self-test for the purity gate (development-guidelines.md §Definition of done:
# "the guard actually guards"). It proves scripts/purity.sh TRIPS on a forbidden dependency edge
# that is genuinely present — deterministically, offline, and without mutating any Cargo.toml — by
# treating tmx-schema (a real normal-dependency of tmx-core and tmx-testkit) as forbidden via
# TMX_PURITY_EXTRA_FORBIDDEN, then confirms the unmodified gate PASSES.
#
# This is the reproducible form of the certificate's tokio-injection check: injecting a real tokio
# edge exercises the identical detection path, but requires a network fetch; using a crate already
# in the tree needs none. A non-zero exit means the guard failed to guard.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PURITY="$ROOT/scripts/purity.sh"

echo "purity selftest: 1/2 — the baseline gate must PASS…"
if ! "$PURITY" >/dev/null; then
  echo "✗ selftest: the baseline purity gate failed but should pass." >&2
  exit 1
fi

echo "purity selftest: 2/2 — an injected forbidden edge (tmx-schema) must FAIL…"
if TMX_PURITY_EXTRA_FORBIDDEN=tmx-schema "$PURITY" >/dev/null 2>&1; then
  echo "✗ selftest: the purity gate PASSED with a forbidden edge present — the guard does not guard." >&2
  exit 1
fi

echo "✓ purity selftest passed: the gate passes clean and trips on a present forbidden edge."
