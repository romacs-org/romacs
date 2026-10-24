#!/usr/bin/env python3
"""Verify mapcan value expectations against the pinned GNU in both regimes."""
import json, os, re, subprocess, sys, tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
TEST = ROOT / "crates/romacs-vm/tests/mapcan.rs"
WRAP = "(prin1-to-string (condition-case e (list 'ok {src}) (error (list 'sig e))))"
ROW = re.compile(r'\(\s*"((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)"\s*\)', re.S)


def unrust(s):
    return re.sub(r'\\(.)', lambda m: {'n': '\n'}.get(m.group(1), m.group(1)), s)


def main():
    sys.path.insert(0, str(ROOT / "scripts"))
    import gnu_oracle
    gnu = gnu_oracle.require()["path"]
    if not os.path.isabs(gnu):
        raise SystemExit("ORACLE must be absolute")
    body = TEST.read_text().split("fn mapcan_matches_gnu_values", 1)[1].split("#[test]", 1)[0]
    rows = [(unrust(a), unrust(b)) for a, b in ROW.findall(body)]
    version = subprocess.run([gnu, "-Q", "--batch", "--eval", "(princ emacs-repository-version)"],
                             capture_output=True, text=True, check=True, stdin=subprocess.DEVNULL).stdout
    print(f"# GNU {gnu}\n# emacs-repository-version {version}\n# rows {len(rows)}")
    bad = 0
    for src, expected in rows:
        form = WRAP.format(src=src)
        prog = ("(let ((f (car (read-from-string (getenv \"MF_FORM\")))))"
                " (princ (format \"%s\\n%s\\n\" (eval f t)"
                " (funcall (byte-compile `(lambda () ,f))))))")
        with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as home:
            r = subprocess.run([gnu, "-Q", "--batch", "--eval", prog], capture_output=True, text=True,
                               env=dict(os.environ, HOME=home, MF_FORM=form), stdin=subprocess.DEVNULL, timeout=60)
        out = r.stdout.splitlines()
        interp, compiled = (out + ["<none>", "<none>"])[:2]
        ok = interp == expected and compiled == expected
        bad += not ok
        print(f"{'OK  ' if ok else 'DIFF'} {src}\n     interpreted {interp}\n     compiled    {compiled}"
              + ("" if ok else f"\n     test says   {expected}"))
    print(f"# {len(rows) - bad}/{len(rows)} rows: test expectation == GNU in both regimes")
    sys.exit(1 if bad else 0)


main()
