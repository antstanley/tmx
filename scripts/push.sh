#!/usr/bin/env bash
# jj-native "pre-push": validate, then push.
#
# Jujutsu does NOT run Git hooks, so `jj git push` bypasses .git/hooks/pre-push.
# Use this wrapper to get the same guarantee under jj (CI remains the real gate).
#
#   scripts/push.sh                 # validate, then `jj git push`
#   scripts/push.sh --all           # extra args are forwarded to `jj git push`
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "pre-push: validating TMX schemas + examples…"
"$ROOT/scripts/validate.sh"
echo "✓ validation passed — pushing with jj"
exec jj git push "$@"
