#!/usr/bin/env python3
"""Strict fresh-VM journey: GNU Custom/faces are deliberately not loaded."""
import gzip
import hashlib
import json
import pathlib
import subprocess
import sys

binary, gnu_root, output, target = map(pathlib.Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
early = ["emacs-lisp/backquote.el", "emacs-lisp/byte-run.el", "subr.el",
         "emacs-lisp/macroexp.el", "emacs-lisp/pcase.el", "emacs-lisp/gv.el",
         "emacs-lisp/inline.el", "keymap.el"]
journey = early + ["international/mule.el", "bindings.el",
                   "emacs-lisp/easy-mmode.el", "emacs-lisp/cl-lib.el",
                   "emacs-lisp/find-func.el", "files.el"]
summary = {}
for label, files in [("without-custom", journey), ("files-independent", early + ["files.el"])]:
    original = b"original\n"
    target.write_bytes(original)
    expressions = [f"(find-file {json.dumps(str(target))})", "(buffer-string)",
                   '(insert "edited")', "(save-buffer)",
                   "(kill-buffer (current-buffer))",
                   f"(find-file {json.dumps(str(target))})", "(buffer-string)"]
    command = [str(binary), str(gnu_root)] + ["lisp/" + f for f in files]
    command += ["--eval=" + expression for expression in expressions]
    result = subprocess.run(command, capture_output=True, text=True, timeout=180)
    rows = [json.loads(line) for line in result.stdout.splitlines()]
    with gzip.open(output / (label + ".jsonl.gz"), "wt") as stream:
        stream.write(result.stdout)
    (output / (label + ".stderr")).write_text(result.stderr)
    last = rows[-1] if rows else {}
    failed = []
    for row in rows:
        if row.get("ok") is False:
            failed = [form for form in row["report"]["steps"] if form["status"] != "ok"]
    summary[label] = {"command": command, "exit": result.returncode,
                      "error": last.get("error"), "failed_forms": failed,
                      "evaluations": [row for row in rows if "eval" in row],
                      "bytes_after_hex": target.read_bytes().hex(),
                      "file_unchanged": target.read_bytes() == original,
                      "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
(output / "journey-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary, indent=2))
