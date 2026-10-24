#!/usr/bin/env python3
"""Augment the from-manifest default-value spec with its domain and probes."""
import json, sys
from pathlib import Path
p = Path(sys.argv[1])
spec = json.loads(p.read_text())
spec["args"] = ["any"]
spec["extra"] = {"0": ["'most-positive-fixnum", "(make-symbol \":primh-u\")"]}
P = "primh-dv"
GC = "(let ((i 0)) (while (< i 5000) (list i i) (setq i (1+ i))))"
spec["probes"] = [
 f"(progn (defvar {P}1 5) (default-value '{P}1))",
 f"(default-value '{P}2)",
 f"(progn (defvar {P}3 1) (with-temp-buffer (make-local-variable '{P}3) (set '{P}3 2) (list (default-value '{P}3) {P}3)))",
 f"(progn (defvar {P}4 1) (list (eval '(let (({P}4 2)) (default-value '{P}4)) t) (default-value '{P}4)))",
 f"(progn (defvar {P}5 1) (make-variable-buffer-local '{P}5) (with-temp-buffer (set '{P}5 3) (eval '(let (({P}5 4)) (list (default-value '{P}5) {P}5)) t)))",
 f"(progn (defvar {P}6 1) (make-variable-buffer-local '{P}6) (with-temp-buffer (eval '(let (({P}6 4)) (list (default-value '{P}6) {P}6)) t)))",
 f"(progn (defvar {P}7-b 7) (defvaralias '{P}7-a '{P}7-b) (default-value '{P}7-a))",
 f"(progn (defvaralias '{P}8-a '{P}8-b) (condition-case e (default-value '{P}8-a) (error e)))",
 f"(list (default-value nil) (default-value t) (default-value :{P}-kw))",
 f"(default-value (make-symbol \":{P}-u\"))",
 f"(progn (defvar {P}9 1) (makunbound '{P}9) (default-value '{P}9))",
 f"(progn (defvar {P}10 (list 1 2)) (eq {P}10 (default-value '{P}10)))",
 f"(progn (defvar {P}11 (list 1 (copy-sequence \"s\"))) {GC} (default-value '{P}11))",
 f"(list (default-value 'most-positive-fixnum) (listp (default-value 'load-path)) (default-value 'case-fold-search))",
 f"(let ((s (make-symbol \"{P}-v\"))) (set s 3) (default-value s))",
 f"(progn (defvar {P}12 1) (with-temp-buffer (make-local-variable '{P}12) (makunbound '{P}12) (default-value '{P}12)))",
 f"(progn (defvaralias '{P}13 :{P}-kw2) (default-value '{P}13))",
 f"(condition-case e (default-value \"{P}\") (error e))",
 f"(list (funcall #'default-value nil) (apply #'default-value '(t)) (funcall 'default-value :{P}-kw3))",
 f"(progn (defvar {P}14 1) (make-variable-buffer-local '{P}14) (with-temp-buffer (set '{P}14 5) (with-temp-buffer (list (default-value '{P}14) {P}14))))",
 f"(progn (defvar {P}15 nil) (makunbound '{P}15) (with-temp-buffer (make-local-variable '{P}15) (set '{P}15 1) (condition-case e (default-value '{P}15) (error e))))",
 f"(progn (defvar {P}16-b 1) (defvaralias '{P}16-a '{P}16-b) (eval '(let (({P}16-a 9)) (default-value '{P}16-b)) t))",
]
p.write_text(json.dumps(spec) + "\n")
print(f"{p}: {len(spec['probes'])} probes")
