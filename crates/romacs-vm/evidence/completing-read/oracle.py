#!/usr/bin/env python3
"""Record only the pinned GNU C completing-read delegation contract."""
import hashlib
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[3]
sys.path.insert(0, str(ROOT / "tools/lib"))
import gnu_oracle

identity = gnu_oracle.require()
spec = (HERE / "spec.json").read_bytes()
rows = json.loads(spec)
for row in rows:
    expression = "(princ (json-serialize (prin1-to-string %s)))" % row["form"]
    run = subprocess.run([identity["path"], "-Q", "--batch", "--eval", expression],
                         capture_output=True, text=True, check=True, timeout=30)
    if not run.stdout.strip():
        raise RuntimeError("GNU emitted no result for " + row["id"])
    row["gnu"] = json.loads(run.stdout)
record = dict(oracle=identity, spec_sha256=hashlib.sha256(spec).hexdigest(),
              recorder_sha256=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
              cases=rows)
(HERE / "gnu.json").write_text(json.dumps(record, indent=2) + "\n")
print(json.dumps(record, indent=2))
