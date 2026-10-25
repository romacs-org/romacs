#!/usr/bin/env python3
"""Replay surgical face mutants, require assertion reds, restore, prove green.

Run only in an owned worktree with the prepared project environment:
  python3 crates/romacs-face/tests/mutants.py --apply --logs "${TMPDIR:-/tmp}/face-mutants"

No gate, benchmark, TRYBUILD overwrite, pipeline, or dependency on scratch inputs.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", required=True)
    parser.add_argument("--logs", type=Path, required=True)
    parser.add_argument("--only")
    parser.add_argument("--results", type=Path)
    args = parser.parse_args()
    if os.environ.get("TRYBUILD"):
        parser.error("TRYBUILD must be unset")
    root = Path(__file__).resolve().parents[3]
    crate = root / "crates/romacs-face"
    manifest = json.loads((crate / "tests/mutants.json").read_text())
    if args.only:
        manifest = [m for m in manifest if m["id"] == args.only]
        if not manifest:
            parser.error("unknown mutant")
    args.logs.mkdir(parents=True, exist_ok=True)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    results = []
    for mutant in manifest:
        files = {crate / e["file"] for e in mutant["edits"]}
        original = {path: path.read_bytes() for path in files}
        before = {str(p.relative_to(crate)): hashlib.sha256(b).hexdigest() for p, b in original.items()}
        command = ["cargo", "test", "-p", "romacs-face", "--test", mutant.get("target", "readiness"), mutant["test"], "--", "--exact", "--nocapture"]
        try:
            for edit in mutant["edits"]:
                path = crate / edit["file"]
                source = path.read_text()
                assert source.count(edit["old"]) == 1, (mutant["id"], "nonunique preimage", edit["file"])
                path.write_text(source.replace(edit["old"], edit["new"], 1))
            red = subprocess.run(command, cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=180)
            (args.logs / (mutant["id"] + "-red.log")).write_text(red.stdout)
            assert red.returncode != 0, (mutant["id"], "survived")
            assert "could not compile `romacs-face`" not in red.stdout, (mutant["id"], "compile failure is not red evidence")
            assert "Finished `test`" in red.stdout, (mutant["id"], "did not compile/run")
            assert mutant["expected"] in red.stdout, (mutant["id"], "wrong assertion", mutant["expected"])
            assert "FAILED" in red.stdout, (mutant["id"], "missing failing test")
        finally:
            for path, data in original.items():
                path.write_bytes(data)
        after = {str(p.relative_to(crate)): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
        assert before == after, (mutant["id"], "restore mismatch")
        green = subprocess.run(command, cwd=root, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=180)
        (args.logs / (mutant["id"] + "-green.log")).write_text(green.stdout)
        assert green.returncode == 0, (mutant["id"], "restored test is not green")
        results.append({"id": mutant["id"], "test": mutant["test"], "red_exit": red.returncode, "expected_assertion": mutant["expected"], "green_exit": green.returncode, "restored": before == after})
        print(f"{mutant['id']}: assertion red {red.returncode}; restored green {green.returncode}", flush=True)
    if args.results:
        args.results.write_text(json.dumps(results, indent=2) + "\n")
    print(f"Killed {len(results)}/{len(manifest)}; all source snapshots restored")


if __name__ == "__main__":
    main()
