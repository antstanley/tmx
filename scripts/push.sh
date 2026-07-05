#!/usr/bin/env bash
# jj-native "pre-push": run the full gate, then push.
#
# Jujutsu does NOT run Git hooks, so `jj git push` bypasses .git/hooks/pre-push.
# Use this wrapper to get the same guarantee under jj (CI remains the real gate).
#
#   scripts/push.sh                 # run the gate, then `jj git push`
#   scripts/push.sh --all           # extra args are forwarded to `jj git push`
#
# The gate (scripts/gate.sh) is: cargo fmt --check · clippy -D warnings · nextest · the dependency
# purity check · the schema/example validation. It is shared with .githooks/pre-push so both paths
# enforce the identical set.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "pre-push: running the full TMX gate (fmt · clippy · nextest · purity · schema)…"
"$ROOT/scripts/gate.sh"
echo "✓ gate passed — pushing with jj"
exec jj git push "$@"
