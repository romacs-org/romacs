"""Re-record the dedicated overlay corpus with the repository's pinned GNU."""
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
EVIDENCE = ROOT / "crates/romacs-vm/evidence/overlay-missing.json"
PIN = "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
exe = Path(os.environ["ROMACS_GNU_ORACLE"])
if not exe.is_absolute():
    raise ValueError("ROMACS_GNU_ORACLE must be absolute")


def evaluate(source):
    return subprocess.check_output(
        [str(exe), "--batch", "-Q", "--eval", source],
        stdin=subprocess.DEVNULL, text=True, timeout=30,
    )


revision = evaluate("(princ emacs-repository-version)")
if revision != PIN:
    raise ValueError(f"GNU revision {revision!r} does not match {PIN}")
evidence = json.loads(EVIDENCE.read_text())
for case in evidence["cases"]:
    case["expected"] = evaluate(f'(prin1 {case["source"]})')
source_dir = Path(json.loads(evaluate("(prin1 source-directory)")))
evidence["revision"] = revision
evidence["binary_sha256"] = hashlib.sha256(exe.read_bytes()).hexdigest()
evidence["buffer_c_sha256"] = hashlib.sha256((source_dir / "src/buffer.c").read_bytes()).hexdigest()
EVIDENCE.write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + "\n")
print(f"Recorded {len(evidence['cases'])} cases against {revision}")
