#!/usr/bin/env python3
"""Prove guard tests reject source defects, never modify the checked-out guard."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
source = (HERE / "master-reference-guard.py").read_text()
mutants = [
    ("disabled-enforcement", 'if state != "prepared":', 'if True:',
     "test_direct_commit_refused", "master move unexpectedly succeeded"),
    ("lost-repository-scope", 'if current != str(Path(pinned).resolve(strict=True)):', 'if False:',
     "test_other_repo_unaffected_even_with_shared_hook", "MASTER GUARD REFUSED:"),
    ("unbound-transaction", 'token != payload(current, moves[0][0], moves[0][1])', 'False',
     "test_wrong_old_new_repository_refused", "master move unexpectedly succeeded"),
    ("extra-ref-authorized", 'len(rows) != 1 or ', '',
     "test_multiref_transaction_refused", "master move unexpectedly succeeded"),
    ("persistent-file-capability", 'fd < 3 or not stat.S_ISFIFO(os.fstat(fd).st_mode)', 'fd < 3',
     "test_regular_file_is_not_capability", "master move unexpectedly succeeded"),
]
with tempfile.TemporaryDirectory(prefix="master-guard-mutants-", dir=os.environ.get("TMPDIR")) as work:
    for name, old, new, test, failure in mutants:
        if source.count(old) != 1:
            raise RuntimeError(f"{name}: mutation anchor is not unique")
        env = dict(os.environ)
        env.pop("MASTER_GUARD_SOURCE", None)
        command = [sys.executable, str(HERE / "test-master-reference-guard.py"), "GuardTests." + test]
        baseline = subprocess.run(command, env=env, text=True, capture_output=True)
        if baseline.returncode or "Ran 1 test" not in baseline.stderr:
            raise RuntimeError(f"{name}: baseline did not pass\n{baseline.stdout}{baseline.stderr}")
        path = Path(work) / (name + ".py")
        path.write_text(source.replace(old, new))
        compile(path.read_text(), str(path), "exec")
        env["MASTER_GUARD_SOURCE"] = str(path)
        result = subprocess.run(command, env=env, text=True, capture_output=True)
        text = result.stdout + result.stderr
        if (result.returncode != 1 or "FAIL: " + test not in text or
                "AssertionError:" not in text or failure not in text or
                "FAILED (failures=1)" not in text):
            raise RuntimeError(f"{name}: no intended assertion failure\n{text}")
        print(json.dumps(dict(mutant=name, test=test, result="KILLED", assertion=failure)))
print(f"MUTANTS PASS: {len(mutants)} source defects rejected by their intended assertions")
