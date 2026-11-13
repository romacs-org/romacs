#!/usr/bin/env python3
"""Render current crate imports without editing provenance-hashed producers.

  render_replay.py primh SPEC... --out TEST.rs
  render_replay.py primh SPEC... --into TEST.rs
  render_replay.py varh --out TEST.rs

The original harness validates evidence and renders its original template;
Only emitted Rust is adapted to the current crate name and value-display API,
and a declaration's Rust location is compared by file, not line: a line moves
whenever code above it does, so two branches that each move it would fail on
their merge though neither is wrong.
`--into` adds each SPEC's replay to TEST.rs, a file rendered from the same
template (tests/primh.rs holds every primitive's), and keeps those it has.
"""
import argparse
import importlib.util
import os
import re
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
PINNED_LINE = '    assert_eq!(spec["rust"], current["rust"], "declaration rust stale");\n'
FILE_ONLY = ('    // The file, not its line: a line moves whenever code above the declaration does, and two\n'
             '    // branches that each move it would fail on their merge though neither is wrong.\n'
             '    let file = |v: &Value| v.as_str().map(|s| s.rsplit_once(\':\').map_or(s, |(f, _)| f).to_owned());\n'
             '    assert_eq!(file(&spec["rust"]), file(&current["rust"]), "declaration rust stale");\n')
REPLAY = re.compile(r'^#\[test\] fn primh_\w+\(\) \{ replay\(.*\); \}$', re.M)


def load_harness(family):
    if family not in ("primh", "varh"):
        raise ValueError(f"unknown harness: {family}")
    directory = HERE if family == "primh" else HERE.parent / "varh"
    spec = importlib.util.spec_from_file_location(family, directory / f"{family}.py")
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load {family}")
    harness = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(harness)
    return harness


def render(family, specs, out):
    if family == "primh" and not specs:
        raise ValueError("primh requires at least one primitive spec")
    if family == "varh" and specs:
        raise ValueError("varh uses its pinned cases.json, not primitive specs")
    harness = load_harness(family)
    harness.rust(argparse.Namespace(spec=specs, out=out))
    path = Path(out)
    text = path.read_text()
    original = "use romacs_vm2::Vm;"
    if text.count(original) != 1:
        raise ValueError("expected exactly one original VM import")
    text = text.replace(original, "use romacs_vm::Vm;", 1)
    text = text.replace(PINNED_LINE, FILE_ONLY, 1)
    path.write_text(text.replace("vm.display(v)", "v.display()"))


def head(text):
    """TEXT up to its first replay line: the rendered template."""
    found = REPLAY.search(text)
    return text[:found.start()] if found else text


def into(specs, target):
    """Add the replays of SPECS that TARGET lacks to TARGET, which must begin
    with the template as rendered now."""
    path = Path(target)
    # Beside TARGET, so the rendered include paths are TARGET's.
    handle, name = tempfile.mkstemp(dir=path.parent, prefix=".render-", suffix=".rs")
    os.close(handle)
    fresh = Path(name)
    try:
        render("primh", specs, str(fresh))
        text = fresh.read_text()
    finally:
        fresh.unlink()
    current = path.read_text()
    if head(current) != head(text):
        raise ValueError(f"{target} does not begin with the template as rendered now")
    present = set(REPLAY.findall(current))
    added = [line for line in REPLAY.findall(text) if line not in present]
    path.write_text(current + "".join(f"{line}\n" for line in added))
    return added


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("family", choices=("primh", "varh"))
    parser.add_argument("spec", nargs="*")
    target = parser.add_mutually_exclusive_group(required=True)
    target.add_argument("--out")
    target.add_argument("--into")
    args = parser.parse_args()
    if args.into and args.family != "primh":
        parser.error("--into adds primitive replays: primh only")
    if args.into:
        into(args.spec, args.into)
    else:
        render(args.family, args.spec, args.out)


if __name__ == "__main__":
    main()
