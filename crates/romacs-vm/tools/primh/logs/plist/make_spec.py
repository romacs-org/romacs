#!/usr/bin/env python3
"""Augment the from-manifest symbol-plist/setplist specs with domains and probes.

  make_spec.py VARS-SPECS-DIR
"""
import json, sys
from pathlib import Path
d = Path(sys.argv[1])

sp = json.loads((d / "symbol-plist.json").read_text())
sp["args"] = ["any"]
sp["extra"] = {"0": ["(make-symbol \"primh-sp\")", "'primh-sp-fresh", ":primh-sp-kw"]}
P = "primh-sp"
sp["probes"] = [
    f"(let ((s (make-symbol \"{P}\"))) (put s 'a 1) (put s 'b 2) (symbol-plist s))",
    f"(let ((a (make-symbol \"{P}\")) (b (make-symbol \"{P}\"))) (put a 'k 1) (list (symbol-plist a) (symbol-plist b) (symbol-plist '{P}-twin)))",
    f"(progn (put '{P}3 'x 1) (put '{P}3 'y 2) (put '{P}3 'x 3) (symbol-plist '{P}3))",
    f"(let ((s (make-symbol \"{P}\")) (l (list 'z 9))) (setplist s l) (eq (symbol-plist s) l))",
    f"(progn (defvar {P}5 1) (make-variable-buffer-local '{P}5) (symbol-plist '{P}5))",
    f"(progn (put :{P}-kw6 'p 1) (symbol-plist :{P}-kw6))",
    f"(progn (put nil '{P}7 1) (plist-get (symbol-plist nil) '{P}7))",
    f"(progn (put t '{P}8 2) (plist-get (symbol-plist t) '{P}8))",
    f"(let ((s (make-symbol \"{P}\"))) (put s 'v (list 1 (list 2 3) (copy-sequence \"s\"))) (let ((i 0)) (while (< i 5000) (list i i) (setq i (1+ i)))) (symbol-plist s))",
]
(d / "symbol-plist.json").write_text(json.dumps(sp) + "\n")

st = json.loads((d / "setplist.json").read_text())
st["args"] = ["any", "any"]
st["extra"] = {"0": ["(make-symbol \"primh-st\")", ":primh-st-kw"], "1": ["(list 'a 1 'b 2)", "(list 'a)"]}
P = "primh-st"
st["probes"] = [
    f"(let ((s (make-symbol \"{P}\")) (l (list 'x 1))) (list (eq (setplist s l) l) (eq (symbol-plist s) l) (get s 'x)))",
    f"(let* ((s (make-symbol \"{P}\")) (l (list 'x 1))) (setplist s l) (put s 'x 2) (put s 'y 3) (list l (eq (symbol-plist s) l)))",
    f"(let ((s (make-symbol \"{P}\"))) (setplist s 5) (list (get s 'a) (symbol-plist s) (condition-case e (put s 'a 1) (error e))))",
    f"(let ((s (make-symbol \"{P}\"))) (setplist s (list 'a)) (list (get s 'a) (condition-case e (put s 'b 1) (error e))))",
    f"(let ((s (make-symbol \"{P}\"))) (setplist s (list 'a 1 'b)) (condition-case e (put s 'c 1) (error e)))",
    f"(let ((s (make-symbol \"{P}\")) (l (list 'a 1))) (setcdr (cdr l) l) (setplist s l) (list (get s 'a) (get s 'z)))",
    f"(let ((s (make-symbol \"{P}\")) (l (list 'a 1))) (setcdr (cdr l) l) (setplist s l) (condition-case e (put s 'z 1) (error (list (car e) (eq (car (cdr e)) l)))))",
    f"(progn (defvar {P}8 1) (make-variable-buffer-local '{P}8) (setplist '{P}8 nil) (local-variable-if-set-p '{P}8))",
    f"(progn (defvar {P}9 1) (with-temp-buffer (make-local-variable '{P}9) (setq {P}9 5) (setplist '{P}9 (list 'q 1)) (list {P}9 (local-variable-if-set-p '{P}9) (get '{P}9 'q))))",
    f"(let ((s (make-symbol \"{P}\"))) (setplist s (list 'k (list 1 (copy-sequence \"v\")))) (let ((i 0)) (while (< i 5000) (list i i) (setq i (1+ i)))) (get s 'k))",
    f"(let ((a (make-symbol \"{P}\")) (b (make-symbol \"{P}\"))) (setplist a (list 'p 1)) (list (get a 'p) (get b 'p)))",
]
(d / "setplist.json").write_text(json.dumps(st) + "\n")
