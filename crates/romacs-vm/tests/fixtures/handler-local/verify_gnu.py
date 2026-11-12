#!/usr/bin/env python3
"""Verify frozen expected values against the pinned GNU binary, not VM2."""
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import tempfile

here = Path(__file__).resolve().parent
pin = json.loads((here / "oracle.json").read_text())
binary = Path(pin["binary"])
assert hashlib.sha256(binary.read_bytes()).hexdigest() == pin["sha256"]
cases = json.loads((here / "cases.json").read_text())
assert len({c["id"] for c in cases}) == len(cases)
with tempfile.TemporaryDirectory(prefix="handler-local-oracle-", dir=os.environ["TMPDIR"]) as directory:
    root = Path(directory)
    for case in cases:
        (root / "input").write_text("hello\n")
        # In one pass: a path holding a placeholder is not rewritten.
        values = {"@DIR@": json.dumps(str(root) + "/"), "@FILE@": json.dumps(str(root / "input")),
                  "@HOME@": json.dumps(os.environ["HOME"])}
        source = re.sub("|".join(map(re.escape, values)), lambda m: values[m.group(0)], case["source"])
        result = subprocess.run([str(binary), "-Q", "--batch", "--eval", "(prin1 " + source + ")"], text=True, capture_output=True, check=True)
        assert result.stdout == case["expected"], (case["id"], result.stdout, case["expected"])
        assert (root / "input").read_text() == case.get("file", "hello\n"), (case["id"], (root / "input").read_text())
print(f"verified {len(cases)} pinned GNU rows")
