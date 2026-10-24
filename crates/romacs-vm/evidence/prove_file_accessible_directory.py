#!/usr/bin/env python3
"""Run pinned GNU permission probes and assertion-keyed production mutants."""
import os as _portable_os
from pathlib import Path as _PortablePath
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
OUT = Path(__file__).resolve().parent / "file-accessible-directory"
SOURCE = ROOT / "crates/romacs-vm/src/prims/file_accessible_directory.rs"
GNU = Path(_portable_os.environ.get('ROMACS_GNU_ORACLE', str(_PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea/bin/emacs-32.0.50')))
PIN = "a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8"


def main():
    OUT.mkdir(exist_ok=True)
    assert hashlib.sha256(GNU.read_bytes()).hexdigest() == PIN
    original = SOURCE.read_text()
    rows = []
    with tempfile.TemporaryDirectory(prefix="accessible-oracle-", dir=os.environ["TMPDIR"]) as tmp:
        base = Path(tmp)
        for name, mode in [("searchable", 0o100), ("unsearchable", 0o400), ("blocked", 0)]:
            (base / name).mkdir()
            (base / name).chmod(mode)
        (base / "file").write_text("")
        (base / "file").chmod(0o700)
        (base / "dir-link").symlink_to(base / "searchable")
        (base / "file-link").symlink_to(base / "file")
        (base / "dangling").symlink_to(base / "absent")
        source = "(progn (setq default-directory " + json.dumps(tmp + "/") + ") (prin1 (list " + " ".join("(file-accessible-directory-p " + json.dumps(name) + ")" for name in ["", ".", "searchable", "searchable/", "unsearchable", "blocked", "file", "dir-link", "file-link", "dangling", "absent"]) + ")))"
        result = subprocess.run([str(GNU), "-Q", "--batch", "--eval", source], text=True, capture_output=True)
        row = dict(phase="live-gnu-permissions", source=source, exit=result.returncode, stdout=result.stdout, stderr=result.stderr)
        rows.append(row)
        try:
            assert result.returncode == 0 and result.stdout == "(t t t t nil nil nil t nil nil nil)", row
        finally:
            for name in ["searchable", "unsearchable", "blocked"]:
                (base / name).chmod(0o700)
    expression = "std::path::Path::new(&searchable).exists()"
    assert original.count(expression) == 1
    mutants = [
        ("plain-existence", "std::path::Path::new(&path).exists()", "accessible-directory directory check: file"),
        ("ignore-directory-check", "std::fs::metadata(&path).is_ok()", "accessible-directory directory check: file"),
        ("ignore-search-permission", "std::path::Path::new(&path).is_dir()", "accessible-directory search permission: unsearchable"),
    ]
    try:
        for phase, text, assertion in [("green", original, None)] + [(name, original.replace(expression, replacement), assertion) for name, replacement, assertion in mutants] + [("restored-green", original, None)]:
            SOURCE.write_text(text)
            command = ["cargo", "test", "-p", "romacs-vm", "--test", "file_accessible_directory", "--", "--nocapture"]
            result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            (OUT / (phase + ".log")).write_text(result.stdout)
            good = result.returncode == 0 and "2 passed;" in result.stdout if assertion is None else result.returncode != 0 and "assertion `left == right` failed: " + assertion in result.stdout and "1 failed;" in result.stdout
            rows.append(dict(phase=phase, command=command, source_sha256=hashlib.sha256(text.encode()).hexdigest(), test_sha256=hashlib.sha256((ROOT / "crates/romacs-vm/tests/file_accessible_directory.rs").read_bytes()).hexdigest(), exit=result.returncode, assertion=assertion, verified=good))
            print(phase, result.returncode, good, flush=True)
            assert good, rows[-1]
    finally:
        SOURCE.write_text(original)
        (OUT / "proof.json").write_text(json.dumps(dict(gnu=str(GNU), gnu_sha256=PIN, base=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), rows=rows, restored_sha256=hashlib.sha256(SOURCE.read_bytes()).hexdigest()), indent=2) + "\n")


if __name__ == "__main__":
    main()
