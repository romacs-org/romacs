#!/usr/bin/env python3
"""Augment the from-manifest default-boundp spec with its domain and probes."""
import json, sys
from pathlib import Path
p = Path(sys.argv[1])
spec = json.loads(p.read_text())
spec["args"] = ["any"]
spec["extra"] = {"0": ["'load-path", "(make-symbol \":primh-u\")"]}
P = "primh-db"
GC = "(let ((i 0)) (while (< i 5000) (list i i) (setq i (1+ i))))"
spec["probes"] = [
 f"(progn (defvar {P}1 5) (default-boundp '{P}1))",
 f"(default-boundp '{P}2)",
 f"(progn (defvar {P}3 nil) (makunbound '{P}3) (with-temp-buffer (make-local-variable '{P}3) (set '{P}3 1) (list (default-boundp '{P}3) (boundp '{P}3))))",
 f"(progn (make-variable-buffer-local '{P}4) (with-temp-buffer (set '{P}4 2) (list (default-boundp '{P}4) (boundp '{P}4))))",
 f"(progn (defvar {P}5 1) (eval '(let (({P}5 2)) (default-boundp '{P}5)) t))",
 f"(progn (defvar {P}6 nil) (makunbound '{P}6) (list (eval '(let (({P}6 2)) (default-boundp '{P}6)) t) (default-boundp '{P}6)))",
 f"(progn (defvar {P}7-b 1) (defvaralias '{P}7-a '{P}7-b) (default-boundp '{P}7-a))",
 f"(progn (defvaralias '{P}8-a '{P}8-b) (list (default-boundp '{P}8-a) (progn (set '{P}8-b 1) (default-boundp '{P}8-a))))",
 f"(list (default-boundp nil) (default-boundp t) (default-boundp :{P}-kw))",
 f"(default-boundp (make-symbol \":{P}-u\"))",
 f"(progn (defvar {P}9 1) (makunbound '{P}9) (default-boundp '{P}9))",
 f"(progn (defvar {P}10 1) (make-variable-buffer-local '{P}10) (with-temp-buffer (set '{P}10 3) (list (default-boundp '{P}10) {P}10)))",
 f"(progn (defvar {P}11 (list 1 (copy-sequence \"s\"))) {GC} (default-boundp '{P}11))",
 f"(list (default-boundp 'load-path) (default-boundp 'most-positive-fixnum) (default-boundp 'case-fold-search))",
 f"(let ((s (make-symbol \"{P}-v\"))) (list (default-boundp s) (progn (set s 1) (default-boundp s))))",
 f"(progn (defvar {P}12 1) (with-temp-buffer (make-local-variable '{P}12) (makunbound '{P}12) (list (default-boundp '{P}12) (boundp '{P}12))))",
 f"(progn (defvaralias '{P}13 :{P}-kw2) (default-boundp '{P}13))",
 f"(progn (defvar {P}14 nil) (makunbound '{P}14) (with-temp-buffer (make-local-variable '{P}14) (list (default-boundp '{P}14) (boundp '{P}14))))",
 f"(progn (defvar {P}15 1) (make-variable-buffer-local '{P}15) (with-temp-buffer (set '{P}15 3) (eval '(let (({P}15 4)) (default-boundp '{P}15)) t)))",
 f"(condition-case e (default-boundp \"{P}\") (error e))",
 f"(list (funcall #'default-boundp nil) (apply #'default-boundp '({P}2)) (funcall 'default-boundp 'load-path))",
]
p.write_text(json.dumps(spec) + "\n")
print(f"{p}: {len(spec['probes'])} probes")
