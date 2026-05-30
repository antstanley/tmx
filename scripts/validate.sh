#!/usr/bin/env bash
# Validate TMX schemas + examples. VCS-agnostic; safe to run anywhere.
# Prefers `uv` (no committed venv); falls back to a local .venv-tmx.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$ROOT/scripts/validate_examples.py"

if command -v uv >/dev/null 2>&1; then
  exec uv run --quiet \
    --with jsonschema --with pyyaml --with json5 --with referencing --with tomli \
    python "$SCRIPT" "$@"
fi

VENV="$ROOT/.venv-tmx"
if [ ! -x "$VENV/bin/python" ]; then
  echo "tmx: creating validation venv at .venv-tmx (first run only)…" >&2
  python3 -m venv "$VENV"
  "$VENV/bin/python" -m pip install --quiet --upgrade pip
  "$VENV/bin/python" -m pip install --quiet jsonschema pyyaml json5 referencing tomli
fi
exec "$VENV/bin/python" "$SCRIPT" "$@"
