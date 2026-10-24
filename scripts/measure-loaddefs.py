#!/usr/bin/env python3
"""Measure an explicit GNU bootstrap before/after partial loaddefs loading.

This is a diagnostic, not a replacement for the standard warm census. It
never skips a form or patches a GNU source. A failed final preload is retained
as a failed chain entry; the existing census adapter then measures the target.
"""
import argparse
import concurrent.futures
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("census", ROOT / "scripts/lisp-census.py")
assert spec is not None and spec.loader is not None
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)
CHAIN = ["emacs-lisp/debug-early", "emacs-lisp/byte-run", "emacs-lisp/backquote",
         "subr", "keymap", "emacs-lisp/pcase", "emacs-lisp/gv", "emacs-lisp/inline", "custom"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--loaddefs", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--after", action="store_true")
    parser.add_argument("--targets-only", action="store_true")
    parser.add_argument("--bootstrap", choices=("gnu", "early-pcase"), default="gnu",
                        help="GNU loadup order (default), or historical diagnostic comparator")
    args = parser.parse_args()
    binary = args.binary.resolve()
    loaddefs = args.loaddefs.resolve()
    pins = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in [binary, loaddefs]}
    baseline = census.fingerprint()
    files = sorted(p.resolve() for p in (ROOT / "gnu-lisp").rglob("*.el"))
    paths = [ROOT / "gnu-lisp"] + sorted(p for p in (ROOT / "gnu-lisp").rglob("*") if p.is_dir())
    chain = (census.bootstrap_chain() if args.bootstrap == "gnu" else
             [ROOT / "gnu-lisp" / (name + ".el") for name in CHAIN])
    if args.after:
        chain.append(loaddefs)
    # Form denominators are the pinned GNU reader's output in the standard
    # baseline, checked against this identical source inventory below.
    baseline_rows = json.loads((args.output.parent / "loaddefs-census-before.json").read_text())["files"]
    denominators = {row["file"]: row for row in baseline_rows}
    assert set(denominators) == {str(p.relative_to(ROOT)) for p in files}
    if args.targets_only:
        targets = {"gnu-lisp/emacs-lisp/cl-lib.el", "gnu-lisp/emacs-lisp/ert.el", "gnu-lisp/files.el",
                   "gnu-lisp/simple.el", "gnu-lisp/minibuffer.el", "gnu-lisp/window.el", "gnu-lisp/subr.el",
                   *{f"gnu-lisp/emacs-lisp/{name}.el" for name in
                     ("rx", "debug", "backtrace", "ewoc", "find-func", "pp", "map")},
                   "gnu-lisp/help-mode.el"}
        files = [p for p in files if str(p.relative_to(ROOT)) in targets]
    rows = []
    raw = args.output.with_suffix(".jsonl")
    with raw.open("x") as out, concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        futures = {pool.submit(census.run_file, path,
                   {"form_count": denominators[str(path.relative_to(ROOT))]["total_forms"], "status": denominators[str(path.relative_to(ROOT))]["gnu_reader_status"], "error": denominators[str(path.relative_to(ROOT))]["gnu_reader_error"]},
                   binary, paths, 20, chain): path for path in files}
        for future in concurrent.futures.as_completed(futures):
            row = future.result()
            rows.append(row)
            out.write(json.dumps(row) + "\n")
            out.flush()
    rows.sort(key=lambda row: row["file"])
    assert baseline == census.fingerprint(), "source changed during measurement"
    assert pins == {name: hashlib.sha256(Path(name).read_bytes()).hexdigest() for name in pins}
    chain_report = census.chain_consensus(rows)
    report = {"scope": "named-bootstrap-targets" if args.targets_only else "whole-tree", "regime": "explicit-targeted-bootstrap-diagnostic", "bootstrap": args.bootstrap, "after_loaddefs": args.after,
              "base_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "source_fingerprint": baseline, "pins": pins, "chain": [str(p) for p in chain],
              "bootstrap_chain": chain_report,
              "files": rows, "summary": {"files_total": len(rows),
              "bootstrap_entries_planned": len(chain),
              "bootstrap_entries_attempted": len(chain_report or []),
              "bootstrap_complete": bool(chain_report) and len(chain_report) == len(chain) and all(entry['loaded'] for entry in chain_report),
              "files_loaded": sum(row["loaded"] for row in rows),
              "forms_evaluated": sum(row["forms_evaluated"] or 0 for row in rows),
              "unavailable": sum(row["forms_evaluated"] is None for row in rows)}}
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["summary"]))


if __name__ == "__main__":
    main()
