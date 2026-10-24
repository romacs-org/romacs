#!/usr/bin/env python3
"""Augment the from-manifest functionp spec with its domain and probes."""
import json, sys
from pathlib import Path
p = Path(sys.argv[1])
spec = json.loads(p.read_text())
spec["args"] = ["any"]
spec["extra"] = {"0": ["'car", "'when", "'if", "(lambda (x) x)", "'(lambda (x) x)", "(list 'autoload \"f\")", "(list 'keymap)"]}
P = "primh-fp"
GC = "(let ((i 0)) (while (< i 5000) (list i i) (setq i (1+ i))))"
spec["probes"] = [
 "(list (functionp 'car) (functionp #'car) (functionp (symbol-function 'car)))",
 "(list (functionp 'when) (functionp 'if) (functionp (symbol-function 'if)) (functionp 'setq) (functionp 'defvar))",
 "(list (functionp 'funcall) (functionp 'apply) (functionp 'functionp))",
 "(list (functionp '(closure (t) (x) x)) (functionp (list 'lambda)) (functionp (cons 'lambda 1)))",
 f"(progn (defun {P}1 () 1) (list (functionp '{P}1) (functionp (symbol-function '{P}1))))",
 f"(progn (defmacro {P}2 () 1) (functionp '{P}2))",
 f"(progn (defalias '{P}3 'car) (functionp '{P}3))",
 f"(progn (defalias '{P}4 '{P}4-missing) (functionp '{P}4))",
 f"(progn (defmacro {P}5m () 1) (defalias '{P}5 '{P}5m) (functionp '{P}5))",
 f"(progn (autoload '{P}6 \"f\") (functionp '{P}6))",
 f"(progn (autoload '{P}7 \"f\" nil nil 'macro) (functionp '{P}7))",
 f"(progn (autoload '{P}8 \"f\" nil nil 'keymap) (functionp '{P}8))",
 f"(progn (autoload '{P}9 \"f\" \"doc\" t nil) (functionp '{P}9))",
 f"(progn (autoload '{P}10b \"f\") (defalias '{P}10 '{P}10b) (functionp '{P}10))",
 f"(list (functionp nil) (functionp t) (functionp :{P}-kw))",
 f"(let ((s (make-symbol \"{P}-u\"))) (fset s (lambda () 1)) (functionp s))",
 f"(functionp '{P}11-unbound)",
 f"(progn (fset '{P}12 nil) (functionp '{P}12))",
 f"(progn (fset '{P}13 1) (functionp '{P}13))",
 f"(progn (fset '{P}14 '(lambda () 1)) (functionp '{P}14))",
 f"(progn (fset '{P}15 (list 'autoload \"f\" nil nil nil)) (functionp '{P}15))",
 f"(progn (fset '{P}16 (list 'autoload \"f\")) (functionp '{P}16))",
 f"(let ((f (lambda (x) x))) {GC} (functionp f))",
 "(list (functionp (eval '(function (lambda () 1)) nil)) (functionp (eval '(lambda () 1) t)))",
 f"(progn (defalias '{P}17 (lambda () 2)) (defalias '{P}18 '{P}17) (defalias '{P}19 '{P}18) (functionp '{P}19))",
 f"(list (funcall #'functionp 'car) (apply #'functionp '(1)))",
 f"(functionp (list 'autoload \"f\" nil nil 'macro))",
 f"(progn (fset '{P}20 (cons 'macro (lambda () 1))) (functionp '{P}20))",
]
p.write_text(json.dumps(spec) + "\n")
print(f"{p}: {len(spec['probes'])} probes")
