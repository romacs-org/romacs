#!/usr/bin/env python3
"""Fresh GNU-owner dependency attempt following check-journey.py's base run."""
import gzip
import hashlib
import json
from pathlib import Path
import subprocess
import sys

binary, gnu_root, output, target = map(Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
base = ["emacs-lisp/backquote.el", "emacs-lisp/byte-run.el", "subr.el",
        "emacs-lisp/macroexp.el", "emacs-lisp/pcase.el", "emacs-lisp/gv.el",
        "emacs-lisp/inline.el", "keymap.el", "international/mule.el", "bindings.el",
        "emacs-lisp/easy-mmode.el", "emacs-lisp/cl-preloaded.el",
        "emacs-lisp/cl-lib.el", "emacs-lisp/find-func.el", "files.el", "uniquify.el"]
target.write_bytes(b"original\n")
expressions = [f"(find-file {json.dumps(str(target))})", "(buffer-string)",
               '(insert "edited")', '(save-buffer)', '(kill-buffer (current-buffer))',
               f"(find-file {json.dumps(str(target))})", '(buffer-string)']
command = [str(binary), str(gnu_root), *["lisp/" + item for item in base],
           *["--eval=" + expression for expression in expressions]]
result = subprocess.run(command, capture_output=True, text=True, timeout=180)
with gzip.open(output / "gnu-owners.jsonl.gz", "wt") as stream:
    stream.write(result.stdout)
(output / "gnu-owners.stderr").write_text(result.stderr)
rows = [json.loads(line) for line in result.stdout.splitlines()]
failures = [{"error": row.get("error"), "failed_forms": [step for step in row.get("report", {}).get("steps", []) if step["status"] != "ok"]} for row in rows if row.get("ok") is False]
evaluations = [row for row in rows if "eval" in row]
summary = {"command": command, "exit": result.returncode, "failures": failures,
           "evaluations": evaluations, "bytes_after_hex": target.read_bytes().hex(),
           "complete": len(evaluations) == len(expressions) and all("Ok" in row["result"] for row in evaluations),
           "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest()}
(output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps(summary, indent=2))
sys.exit(0 if summary["complete"] else 1)
