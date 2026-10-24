#!/usr/bin/env python3
"""Render current crate imports without editing provenance-hashed producers.

  render_replay.py primh SPEC... --out TEST.rs
  render_replay.py varh --out TEST.rs

The original harness validates evidence and renders its original template;
Only emitted Rust is adapted to the current crate name and value-display API.
"""
import argparse
import importlib.util
from pathlib import Path

HERE = Path(__file__).resolve().parent


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
    path.write_text(text.replace("vm.display(v)", "v.display()"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("family", choices=("primh", "varh"))
    parser.add_argument("spec", nargs="*")
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    render(args.family, args.spec, args.out)


if __name__ == "__main__":
    main()
