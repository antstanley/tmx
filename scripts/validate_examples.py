#!/usr/bin/env python3
"""Validate TMX schemas and example documents.

Usage:
  validate_examples.py            # meta-validate docs/*.schema.json, validate every
                                  # instance under docs/examples/**, check format parity
  validate_examples.py FILE ...   # validate only the given instance files

Each instance is dispatched to the right schema by its `kind`
(flow | environment | context | task | provider), falling back to filename/shape.
Exit code 0 on success, 1 if any check fails.
"""
import sys
import json
from pathlib import Path

try:
    import tomllib  # Python 3.11+
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore
import yaml
try:
    import json5
    HAVE_JSON5 = True
except ModuleNotFoundError:  # pragma: no cover
    HAVE_JSON5 = False
from jsonschema import Draft202012Validator
from referencing import Registry, Resource

ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs"
EXAMPLES = DOCS / "examples"
INSTANCE_EXTS = ("*.json", "*.jsonc", "*.json5", "*.yaml", "*.yml", "*.toml")

failures = 0


def rel(p: Path) -> str:
    p = Path(p).resolve()
    try:
        return str(p.relative_to(ROOT))
    except ValueError:
        return str(p)


def report(ok: bool, label: str) -> None:
    global failures
    print(f"[{'PASS' if ok else 'FAIL'}] {label}")
    if not ok:
        failures += 1


def load_instance(path: Path):
    text = Path(path).read_text()
    suffix = Path(path).suffix
    if suffix in (".yaml", ".yml"):
        return yaml.safe_load(text)
    if suffix == ".toml":
        return tomllib.loads(text)
    if suffix in (".jsonc", ".json5"):
        if not HAVE_JSON5:
            raise RuntimeError("json5 is required to parse .jsonc/.json5 files")
        return json5.loads(text)
    return json.loads(text)


# ---- build a registry from every *.schema.json in docs/ ----
schema_files = sorted(DOCS.glob("*.schema.json"))
if not schema_files:
    print("no *.schema.json found under docs/", file=sys.stderr)
    sys.exit(1)

resources = []
by_name = {}
for sf in schema_files:
    data = json.loads(sf.read_text())
    sid = data.get("$id", sf.name)
    resources.append((sid, Resource.from_contents(data)))
    by_name[sf.name] = data
registry = Registry().with_resources(resources)

main = by_name["tmx.schema.json"]
provider = by_name["tmx-provider.schema.json"]


def validator_for_kind(kind):
    if kind in (None, "flow"):
        return Draft202012Validator(main, registry=registry)
    if kind == "provider":
        return Draft202012Validator(provider, registry=registry)
    if kind in ("environment", "context", "task"):
        sub = {
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": f"#/$defs/{kind}",
            "$defs": main["$defs"],
        }
        return Draft202012Validator(sub, registry=registry)
    return None


def infer_kind(doc, path: Path):
    if isinstance(doc, dict) and "kind" in doc:
        return doc["kind"]
    name = Path(path).name.lower()
    if name.startswith("environment"):
        return "environment"
    if name.startswith("context"):
        return "context"
    if name.startswith("task"):
        return "task"
    if isinstance(doc, dict) and "methods" in doc:
        return "provider"
    return "flow"


# ---- 1. meta-validate the schemas themselves ----
for sf in schema_files:
    try:
        Draft202012Validator.check_schema(json.loads(sf.read_text()))
        report(True, f"meta-schema  {rel(sf)}")
    except Exception as exc:  # noqa: BLE001
        report(False, f"meta-schema  {rel(sf)} :: {exc}")

# ---- 2. validate instances ----
explicit = sys.argv[1:]
if explicit:
    targets = [Path(a) for a in explicit]
else:
    targets = sorted({p for ext in INSTANCE_EXTS for p in EXAMPLES.rglob(ext)})

for t in targets:
    try:
        doc = load_instance(t)
    except Exception as exc:  # noqa: BLE001
        report(False, f"parse        {rel(t)} :: {exc}")
        continue
    kind = infer_kind(doc, t)
    validator = validator_for_kind(kind)
    if validator is None:
        report(False, f"validate     {rel(t)} :: unknown kind '{kind}'")
        continue
    errors = sorted(validator.iter_errors(doc), key=lambda e: list(e.path))
    report(not errors, f"validate     {rel(t)}  (kind={kind})")
    for e in errors[:6]:
        print("     -", list(e.path), e.message)

# ---- 3. cross-format parity for single-file-flow.* ----
if not explicit:
    base_path = EXAMPLES / "single-file-flow.json"
    if base_path.exists():
        base = load_instance(base_path)
        for g in sorted(EXAMPLES.glob("single-file-flow.*")):
            if g == base_path:
                continue
            try:
                same = load_instance(g) == base
            except Exception:  # noqa: BLE001
                same = False
            report(same, f"parity       {g.name} == single-file-flow.json")

print()
if failures:
    print(f"✗ {failures} check(s) failed")
    sys.exit(1)
print("✓ all checks passed")
