#!/usr/bin/env python3
"""Read-only primitive probes; fixture creation is explicit and test-owned."""
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
PINS = json.loads((HERE / "pins.json").read_text())
for filename, expected in PINS.items():
    assert hashlib.sha256(Path(filename).read_bytes()).hexdigest() == expected, filename
BINARY = next(filename for filename in PINS if filename.endswith("emacs-32.0.50"))
with tempfile.TemporaryDirectory(prefix="savejourney-gnu-case-", dir=os.environ["TMPDIR"]) as root:
    root = Path(root)
    (root / "populated").mkdir()
    (root / "populated" / "a").write_bytes(b"unchanged")
    (root / "empty").mkdir()
    (root / "hardlinks").mkdir()
    (root / "hardlinks" / "a").write_bytes(b"unchanged")
    os.link(root / "hardlinks" / "a", root / "hardlinks" / "A")
    endpoint = socket.socket(socket.AF_UNIX)
    endpoint.bind(str(root / "socket"))
    q = lambda value: json.dumps(str(value))
    cases = {
        "file": f'(file-name-case-insensitive-p {q(root / "populated" / "a")})',
        "directory": f'(file-name-case-insensitive-p {q(root / "populated")})',
        "nonexistent": f'(file-name-case-insensitive-p {q(root / "populated" / "missing")})',
        "empty-directory": f'(file-name-case-insensitive-p {q(root / "empty")})',
        "hardlink-alternate": f'(file-name-case-insensitive-p {q(root / "hardlinks" / "a")})',
        "socket": f'(file-name-case-insensitive-p {q(root / "socket")})',
        "invalid-input": '(file-name-case-insensitive-p 2)',
        "handler": '(progn (defun sj-handler (op name) (list op name)) (put \'sj-handler \'operations \'(file-name-case-insensitive-p)) (let ((default-directory "/tmp/") (file-name-handler-alist (list (cons "case-probe" \'sj-handler)))) (file-name-case-insensitive-p "case-probe")))',
    }
    rows = []
    for name, form in cases.items():
        command = [BINARY, '-Q', '--batch', '--eval', f'(prin1 (condition-case e {form} (error e)))']
        result = subprocess.run(command, capture_output=True, text=True, check=True)
        rows.append(dict(id=name, form=form, stdout=result.stdout, stderr=result.stderr, returncode=result.returncode))
    endpoint.close()
(HERE / "gnu-case-detection.json").write_text(json.dumps(dict(binary=BINARY, pins=PINS, cases=rows), indent=2) + '\n')
print(json.dumps(rows, indent=2))
