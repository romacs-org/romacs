#!/usr/bin/env python3
"""Record only GNU C-owned filename semantics, with an exact source pin."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

PIN = "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
GNU = Path(os.environ["ROMACS_GNU_ORACLE"])
ROOT = Path(__file__).resolve().parents[2]
SPECS = ROOT / "tools/primh/filename-native-cases.json"
OUT = ROOT / "evidence/filename-native-gnu.json"

def main():
    source = Path(os.environ.get("ROMACS_GNU_SOURCE", str(GNU.parents[2] / "src")))
    actual = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    assert actual == PIN, actual
    cases = json.loads(SPECS.read_text())
    rows = []
    with tempfile.TemporaryDirectory(prefix="filename-native-", dir=os.environ["TMPDIR"]) as directory:
        fixture = Path(directory)
        for name in ["zeta", "alpha", "a.el", ".hidden", "é"]:
            (fixture / name).write_text("")
        (fixture / "sub").mkdir()
        for case in cases:
            expr = case["source"].replace("@ROOT@", directory)
            setup = "(require 'env)" if case.get("real_helper") else "(fmakunbound 'substitute-env-in-file-name)"
            program = "(progn " + setup + " (prin1 (condition-case e " + expr + " (error e))))"
            env = dict(os.environ, LC_ALL="C.UTF-8", LANG="C.UTF-8")
            run = subprocess.run([str(GNU), "-Q", "--batch", "--eval", program], env=env, text=True, capture_output=True, check=True)
            assert run.stdout.strip(), run.stderr
            rows.append(dict(case, expected=run.stdout.strip().replace(directory, "@ROOT@")))
    result = {"gnu_commit": PIN, "gnu_binary": GNU.name, "binary_sha256": hashlib.sha256(GNU.read_bytes()).hexdigest(), "scope": "C-owned semantics with explicit unbound, spy, and real GNU env.el helper cases", "cases": rows}
    OUT.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(f"recorded {len(rows)} cases to {OUT}")

if __name__ == "__main__":
    main()
