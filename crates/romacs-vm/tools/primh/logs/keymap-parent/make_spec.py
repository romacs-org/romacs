#!/usr/bin/env python3
"""Augment the from-manifest keymap-parent / set-keymap-parent specs with
their domains and probes.  Run once per spec path."""
import json, sys
from pathlib import Path
# Keymap shapes added to every position's catalog: a fresh sparse map, one
# with a prompt, one with a binding, a map that already has a parent (as a
# keymap tail), a bare `(keymap)' list, a symbol whose function cell is a
# keymap, and a list whose head is not `keymap'.
KEYMAPS = [
    "(make-sparse-keymap)",
    "(make-sparse-keymap \"P\")",
    "(list 'keymap (cons 97 'primh-a))",
    "(let ((m (make-sparse-keymap)) (p (list 'keymap (cons 98 'primh-b)))) (setcdr m p) m)",
    "(list 'keymap)",
    "(progn (fset 'primh-km (list 'keymap (cons 99 'primh-c))) 'primh-km)",
    "(list 'notkeymap 1)",
]
GC = "(let ((i 0)) (while (< i 5000) (list i i) (setq i (1+ i))))"
PROBES = {
 "keymap-parent": [
  "(let ((m (make-sparse-keymap)) (p (make-sparse-keymap))) (set-keymap-parent m p) (eq (keymap-parent m) p))",
  "(let ((m (list 'keymap (cons 97 'x))) (p (list 'keymap (cons 98 'y)))) (set-keymap-parent m p) (keymap-parent m))",
  "(let ((m (make-sparse-keymap \"P\")) (p (make-sparse-keymap))) (set-keymap-parent m p) (list m (eq (keymap-parent m) p)))",
  "(progn (fset 'primh-kp1 (list 'keymap (cons 97 'x) 'keymap)) (keymap-parent 'primh-kp1))",
  "(progn (fset 'primh-kp2 (list 'keymap)) (keymap-parent (list 'keymap (cons 1 2) 'primh-kp2 3)))",
  "(keymap-parent (cons 'keymap (cons (cons 1 2) 'primh-kp-missing)))",
  "(progn (fset 'primh-kp3 (make-sparse-keymap)) (keymap-parent (cons 'keymap 'primh-kp3)))",
  "(progn (fset 'primh-kp4 'primh-kp5) (fset 'primh-kp5 (list 'keymap)) (keymap-parent (cons 'keymap 'primh-kp4)))",
  "(keymap-parent (cons 'keymap 7))",
  "(let ((a (make-sparse-keymap)) (b (make-sparse-keymap)) (c (make-sparse-keymap))) (set-keymap-parent a b) (set-keymap-parent b c) (list (eq (keymap-parent a) b) (eq (keymap-parent (keymap-parent a)) c) (keymap-parent c)))",
  f"(let ((m (make-sparse-keymap)) (p (list 'keymap (copy-sequence \"s\")))) (set-keymap-parent m p) {GC} (list (eq (keymap-parent m) p) (keymap-parent m)))",
  "(condition-case e (keymap-parent 'primh-kp-unbound) (error e))",
  "(list (funcall #'keymap-parent (list 'keymap 'keymap)) (apply #'keymap-parent (list (list 'keymap))))",
 ],
 "set-keymap-parent": [
  "(let ((m (make-sparse-keymap)) (p (make-sparse-keymap))) (list (eq (set-keymap-parent m p) p) m))",
  "(let ((m (list 'keymap (cons 97 'x))) (p (list 'keymap (cons 98 'y)))) (set-keymap-parent m p) m)",
  "(let ((m (make-sparse-keymap \"P\")) (p (make-sparse-keymap))) (set-keymap-parent m p) m)",
  "(let ((a (make-sparse-keymap)) (b (make-sparse-keymap))) (set-keymap-parent a b) (condition-case e (set-keymap-parent b a) (error e)))",
  "(let ((a (make-sparse-keymap))) (condition-case e (set-keymap-parent a a) (error e)))",
  "(let ((a (make-sparse-keymap)) (b (make-sparse-keymap)) (c (make-sparse-keymap))) (set-keymap-parent a b) (set-keymap-parent b c) (list (condition-case e (set-keymap-parent c a) (error e)) (condition-case e (set-keymap-parent c b) (error e))))",
  "(let ((m (make-sparse-keymap)) (p (make-sparse-keymap)) (q (list 'keymap (cons 1 2)))) (set-keymap-parent m p) (set-keymap-parent m q) (list m (eq (cdr m) q)))",
  "(let ((m (list 'keymap (cons 97 'x))) (p (make-sparse-keymap))) (set-keymap-parent m p) (set-keymap-parent m nil) m)",
  "(progn (fset 'primh-skp1 (list 'keymap (cons 99 'z))) (let ((m (make-sparse-keymap))) (list (eq (set-keymap-parent m 'primh-skp1) (symbol-function 'primh-skp1)) m)))",
  "(progn (fset 'primh-skp2 (make-sparse-keymap)) (let ((p (make-sparse-keymap))) (list (set-keymap-parent 'primh-skp2 p) (symbol-function 'primh-skp2))))",
  "(let ((m (cons 'keymap (cons (cons 1 2) 7))) (p (make-sparse-keymap))) (set-keymap-parent m p) m)",
  "(progn (fset 'primh-skp3 (list 'keymap)) (let ((m (list 'keymap (cons 1 2) 'primh-skp3)) (p (list 'keymap (cons 3 4)))) (set-keymap-parent m p) m))",
  "(condition-case e (set-keymap-parent 'primh-skp-unbound (make-sparse-keymap)) (error e))",
  "(condition-case e (set-keymap-parent (make-sparse-keymap) 'primh-skp-unbound2) (error e))",
  "(condition-case e (set-keymap-parent (make-sparse-keymap) (list 'notkeymap)) (error e))",
  "(let ((a (make-sparse-keymap)) (b (make-sparse-keymap))) (fset 'primh-skp4 a) (set-keymap-parent a b) (condition-case e (set-keymap-parent b 'primh-skp4) (error e)))",
  f"(let ((m (make-sparse-keymap)) (p (list 'keymap (copy-sequence \"s\")))) (set-keymap-parent m p) {GC} (list (eq (cdr m) p) m))",
  "(list (funcall #'set-keymap-parent (make-sparse-keymap) nil) (apply #'set-keymap-parent (list (make-sparse-keymap) (list 'keymap))))",
  "(let ((m (make-sparse-keymap))) (set-keymap-parent m (make-sparse-keymap)) (define-key m \"a\" 'primh-x) m)",
 ],
}
p = Path(sys.argv[1])
spec = json.loads(p.read_text())
n = spec["arity"][0]
spec["args"] = ["any"] * n
spec["extra"] = {str(i): KEYMAPS for i in range(n)}
spec["probes"] = PROBES[spec["name"]]
p.write_text(json.dumps(spec) + "\n")
print(f"{p}: {len(spec['probes'])} probes, {len(KEYMAPS)} keymap shapes per position")
