#!/usr/bin/env python3
"""Augment the from-manifest set-default spec with its domain and probes."""
import json, sys
from pathlib import Path
p = Path(sys.argv[1])
spec = json.loads(p.read_text())
# SYMBOL: every catalog symbol plus fresh and built-in ones; VALUE: a few
# shapes (the value is stored, never inspected).
spec["args"] = ["symbol", "atom"]
spec["extra"] = {"0": ["1", "(copy-sequence \"s\")", "(list 1 2)", "'primh-sd-fresh", "(make-symbol \":primh-u\")", "'most-positive-fixnum"],
                 "1": [":kw", "(list 1 2)"]}
P = "primh-sd"
GC = "(let ((i 0)) (while (< i 5000) (list i i) (setq i (1+ i))))"
spec["probes"] = [
 f"(list (set-default '{P}1 5) (symbol-value '{P}1) (default-value '{P}1))",
 f"(progn (defvar {P}2 1) (with-temp-buffer (make-local-variable '{P}2) (set '{P}2 2) (list (set-default '{P}2 3) {P}2 (default-value '{P}2))))",
 f"(progn (defvar {P}3 1) (with-temp-buffer (make-local-variable '{P}3) (set '{P}3 2) (set-default '{P}3 3)) {P}3)",
 f"(progn (defvar {P}4 1) (list (eval '(let (({P}4 2)) (set-default '{P}4 3) (list {P}4 (default-value '{P}4))) t) {P}4))",
 f"(progn (defvar {P}5 1) (make-variable-buffer-local '{P}5) (with-temp-buffer (set '{P}5 3) (eval '(let (({P}5 4)) (set-default '{P}5 7) (list (default-value '{P}5) {P}5)) t)))",
 f"(progn (defvar {P}6 1) (make-variable-buffer-local '{P}6) (list (with-temp-buffer (eval '(let (({P}6 4)) (set-default '{P}6 8) (list (default-value '{P}6) {P}6)) t)) {P}6))",
 f"(progn (defvar {P}7-b 7) (defvaralias '{P}7-a '{P}7-b) (set-default '{P}7-a 9) (list {P}7-b (default-value '{P}7-a)))",
 f"(progn (defvaralias '{P}8-a '{P}8-b) (set-default '{P}8-a 1) (list (symbol-value '{P}8-b)))",
 "(condition-case e (set-default nil 1) (error e))",
 "(condition-case e (set-default nil nil) (error e))",
 "(condition-case e (set-default t t) (error e))",
 f"(list (set-default :{P}-kw :{P}-kw) (condition-case e (set-default :{P}-kw 1) (error e)))",
 f"(let ((s (make-symbol \":{P}-u\"))) (list (set-default s 1) (symbol-value s)))",
 f"(progn (defvaralias '{P}9 :{P}-kw2) (list (condition-case e (set-default '{P}9 :{P}-kw2) (error e)) (condition-case e (set-default '{P}9 1) (error e))))",
 "(condition-case e (set-default 'most-positive-fixnum 1) (error e))",
 f"(let ((v (list 1 (copy-sequence \"s\")))) (set-default '{P}10 v) {GC} (list (eq v (default-value '{P}10)) (default-value '{P}10)))",
 f"(let ((s (make-symbol \"{P}-v\"))) (set-default s 2) (list (symbol-value s) (default-value s)))",
 f"(progn (defvar {P}11 1) (make-variable-buffer-local '{P}11) (with-temp-buffer (set '{P}11 5) (set-default '{P}11 6) (with-temp-buffer (list {P}11 (default-value '{P}11)))))",
 f"(condition-case e (set-default \"{P}\" 1) (error e))",
 f"(list (funcall #'set-default '{P}12 1) (apply #'set-default '({P}13 2)) {P}12 {P}13)",
 f"(progn (defvar {P}14 nil) (makunbound '{P}14) (with-temp-buffer (make-local-variable '{P}14) (set '{P}14 1) (set-default '{P}14 2) (list {P}14 (default-value '{P}14))))",
 f"(progn (defvar {P}15 1) (list (eval '(let (({P}15 2)) (set-default '{P}15 3)) t) {P}15))",
 f"(progn (set-default '{P}16 1) (special-variable-p '{P}16))",
 f"(progn (defvar {P}17 (list 0)) (set-default '{P}17 (cons 1 {P}17)) {P}17)",
]
p.write_text(json.dumps(spec) + "\n")
print(f"{p}: {len(spec['probes'])} probes")
