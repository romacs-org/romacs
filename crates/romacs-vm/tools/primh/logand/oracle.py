#!/usr/bin/env python3
"""Record exact logand values and condition data using the pinned GNU oracle."""
import hashlib
import json
import pathlib
import subprocess
import sys
ROOT = pathlib.Path(__file__).resolve().parents[5]
sys.path.insert(0, str(ROOT / "scripts"))
import gnu_oracle
HERE = pathlib.Path(__file__).resolve().parent
ident = gnu_oracle.require()
spec = (HERE / "cases.json").read_bytes()
cases = json.loads(spec)
for case in cases:
    form = "(condition-case e %s (error e))" % case["expression"]
    case["form"] = form
    expr = "(princ (json-serialize (prin1-to-string %s)))" % form
    run = subprocess.run([ident["path"], "-Q", "--batch", "--eval", expr], capture_output=True, text=True, check=True, timeout=30)
    case["gnu"] = json.loads(run.stdout)
record = dict(protocol="logand-v1", oracle=ident, cases_sha256=hashlib.sha256(spec).hexdigest(), recorder_sha256=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(), cases=cases)
(HERE / "gnu.json").write_text(json.dumps(record, indent=2) + "\n")
print(f"Recorded {len(cases)} cases from {ident['revision']}")
