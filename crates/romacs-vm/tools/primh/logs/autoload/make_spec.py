#!/usr/bin/env python3
"""Augment the from-manifest autoload spec with its domain and probes."""
import json, sys
from pathlib import Path
p = Path(sys.argv[1])
spec = json.loads(p.read_text())
# Five positions; keep the cartesian product near 360 rows by giving the
# optional DOCSTRING/INTERACTIVE/TYPE two values each (float tag = 1.5).
# TYPE avoids 'macro in the cartesian rows: GNU's byte compiler evaluates a
# constant macro autoload at COMPILE time (byte-compile-autoload), so a bad
# FUNCTION there escapes the case as a compiler error, which primh refuses as
# HARNESS-ERROR. Valid macro autoloads are covered by probes.
spec["args"] = ["symbol", "string", "float", "float", "float"]
spec["extra"] = {"0": ["1", "'primh-al-f", "'car", "(copy-sequence \"s\")", "'primh-al-g"],
                 "1": ["nil", "1"], "2": ["(copy-sequence \"doc\")"], "3": ["t"], "4": ["'keymap"]}
P = "primh-al"
spec["probes"] = [
 f"(progn (autoload '{P}1 \"{P}-file\" \"Doc\" t) (list (symbol-function '{P}1) (fboundp '{P}1)))",
 f"(progn (autoload '{P}2 \"f1\") (list (autoload '{P}2 \"f2\" nil nil 'macro) (symbol-function '{P}2)))",
 f"(progn (defalias '{P}3 (lambda () 1)) (list (autoload '{P}3 \"f\") (funcall '{P}3)))",
 f"(progn (fset '{P}4 nil) (list (autoload '{P}4 \"f\") (symbol-function '{P}4)))",
 "(autoload nil \"f\")",
 f"(progn (autoload '{P}7 \"{P}-no-such-file\") (condition-case e ({P}7) (error (car e))))",
 f"(progn (autoload '{P}9 \"f\") (eq (symbol-function '{P}9) (symbol-function '{P}9)))",
 f"(progn (autoload '{P}10 \"f\" nil nil 'keymap) (symbol-function '{P}10))",
 f"(eq '{P}11 (autoload '{P}11 \"f\"))",
 f"(progn (autoload '{P}12 \"f\") (defalias '{P}12 (lambda () 2)) (funcall '{P}12))",
 f"(progn (autoload '{P}14 \"f\" 42 'x 'y) (symbol-function '{P}14))",
 f"(progn (autoload '{P}16 (copy-sequence \"file\")) (let ((i 0)) (while (< i 5000) (list i i) (setq i (1+ i)))) (symbol-function '{P}16))",
 f"(let ((s (make-symbol \"{P}-uninterned\"))) (list (eq s (autoload s \"f\")) (symbol-function s)))",
 f"(progn (defalias '{P}18 '{P}18-target) (list (autoload '{P}18 \"f\") (symbol-function '{P}18)))",
 f"(progn (autoload '{P}19 \"f\") (autoload '{P}19 \"g\") (nth 1 (symbol-function '{P}19)))",
 f"(let ((f (copy-sequence \"shared\"))) (autoload '{P}20 f) (eq f (nth 1 (symbol-function '{P}20))))",
 "(autoload 'car \"f\")",
 f"(progn (autoload '{P}22 \"f\" nil nil 'macro) (symbol-function '{P}22))",
]
p.write_text(json.dumps(spec) + "\n")
print(f"{p}: {len(spec['probes'])} probes")
