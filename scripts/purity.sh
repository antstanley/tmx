#!/usr/bin/env bash
# Purity gate for the TMX hexagon.
#
# The pure crates — tmx-schema, tmx-core, tmx-testkit — must never gain an I/O or async dependency
# edge: the core is deterministic and side-effect-free, and tmx-testkit fakes it inside the same
# boundary (02-crate-architecture.md §Dependency graph, architecture-principles.md §1). This script
# guards that boundary at the dependency level so it cannot silently rot as adapters are added.
#
# A non-zero exit names the offending crate and edge. Invoked by scripts/gate.sh (the pre-push
# gate) and runnable directly in CI. VCS-agnostic; safe to run anywhere.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Crates that MUST stay pure. Mirrored by PURE_CRATES in crates/tmx-cli/tests/purity.rs.
PURE_CRATES=(tmx-schema tmx-core tmx-testkit)

# Forbidden dependency-tree package names. tokio is the async runtime; reqwest the HTTP client; the
# aws-sdk-s3 / rust-s3 / rusoto_s3 / object_store family the object store. Any of these in a pure
# crate's normal dependency tree is an inward I/O edge the core must not have. (std::process is not
# a crate and cannot appear in cargo tree — it is caught by the source scan below.)
FORBIDDEN_CRATES=(tokio reqwest aws-sdk-s3 rust-s3 rusoto_s3 object_store)

# Optional extra forbidden names (space-separated) — used by scripts/purity_selftest.sh to prove the
# guard trips on a forbidden edge that is genuinely present, offline and without mutating any file.
if [ -n "${TMX_PURITY_EXTRA_FORBIDDEN:-}" ]; then
  # shellcheck disable=SC2206  # deliberate word-split: the variable is a space-separated name list.
  FORBIDDEN_CRATES+=(${TMX_PURITY_EXTRA_FORBIDDEN})
fi

status=0

for crate in "${PURE_CRATES[@]}"; do
  # Normal-edge dependency tree (dev/build deps excluded), with the root line dropped so a crate is
  # never flagged against its own name. `|| true` absorbs grep's exit 1 when the filter empties the
  # list (a crate with no dependencies), which is a clean pass, not an error.
  deps="$(cargo tree --package "$crate" --edges normal --prefix none 2>/dev/null \
    | awk 'NF {print $1}' | grep -vxF "$crate" | sort -u || true)"
  for bad in "${FORBIDDEN_CRATES[@]}"; do
    if grep -qxF "$bad" <<<"$deps"; then
      echo "✗ purity: crate '$crate' has a forbidden I/O/async dependency edge on '$bad'" >&2
      status=1
    fi
  done
done

# std::process is std, not a crate, so cargo tree cannot see it: scan the pure crates' sources.
# Spawning or exiting a process is the ProcessRunner adapter's job, behind a port — never the core's.
# `//`-comments (doc comments included) are stripped before matching, so prose stating the rule —
# tmx-core's own crate docs literally say "no std::process" — is never a false positive.
for crate in "${PURE_CRATES[@]}"; do
  src="crates/$crate/src"
  [ -d "$src" ] || continue
  if find "$src" -name '*.rs' -type f -exec awk '{ sub(/\/\/.*/, ""); print }' {} + \
    | grep -qE 'std::process|process::Command'; then
    echo "✗ purity: crate '$crate' uses std::process in its source code" >&2
    status=1
  fi
done

if [ "$status" -ne 0 ]; then
  echo "purity gate FAILED — a pure crate reached across the I/O boundary." >&2
  exit 1
fi

echo "✓ purity: tmx-schema, tmx-core, tmx-testkit carry no I/O or async dependency edge."
