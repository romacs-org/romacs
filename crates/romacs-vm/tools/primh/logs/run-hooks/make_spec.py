#!/usr/bin/env python3
"""Augment the from-manifest run-hooks spec (GNU eval.c:2883 over
run_hook_with_args, eval.c:2998-3062) with probes.  Inputs only: every answer
is recorded from live GNU.  Each probe defines its hook functions afresh, runs
`run-hooks' under `condition-case' and returns (RESULT-OR-ERROR LOG), LOG
being the hook functions that ran, in order.  The log lives in the global
value of `rh-log' (read and written with `set'/`symbol-value', so the probe
means the same byte-compiled and interpreted)."""
import json, sys
from pathlib import Path

def rec(tag):
    return f"(set 'rh-log (cons {tag} (symbol-value 'rh-log)))"

REC = {t: rec("'" + t) for t in ("a", "b", "c", "err", "gc")}
DEFS = ("(set 'rh-log nil)"
        f" (defalias 'rh-a (lambda () {REC['a']} (list 'ret-a)))"
        f" (defalias 'rh-b (lambda () {REC['b']} t))"
        f" (defalias 'rh-c (lambda () {REC['c']} nil))"
        f" (defalias 'rh-err (lambda () {REC['err']} (error \"rh-boom\")))")

def run(setup, call="(run-hooks 'rh-h)"):
    return (f"(progn {DEFS} {setup} (list (condition-case e {call} (error e))"
            " (nreverse (symbol-value 'rh-log))))")

def local(global_setup, local_value, var="rh-h"):
    """VAR made buffer-local (as add-hook LOCAL would) with LOCAL_VALUE."""
    return run(global_setup,
               f"(with-temp-buffer (make-local-variable '{var}) (set '{var} {local_value}) (run-hooks '{var}))")

GC = "(let ((i 0)) (while (< i 3000) (list i (make-string 3 ?x) (vector i)) (setq i (1+ i))))"

PROBES = [
    # Arguments: none, non-symbols, nil/t/keyword, unbound and nil hooks.
    "(run-hooks)",
    "(list (run-hooks) (funcall #'run-hooks) (apply #'run-hooks nil))",
    run("", "(run-hooks 5)"),
    run("", "(run-hooks \"x\")"),
    run("", "(run-hooks 1.5)"),
    run("", "(run-hooks '(rh-a))"),
    run("", "(run-hooks nil)"),
    run("", "(run-hooks t)"),
    run("", "(run-hooks :rh-key)"),
    run("", "(run-hooks 'rh-never-bound-hook)"),
    run("(set 'rh-h nil)"),
    run("(set 'rh-h nil)", "(boundp 'run-hooks)"),
    # A value that is not a list, or is a function, is called as a whole.
    run("(set 'rh-h 'rh-a)"),
    run("(set 'rh-h (lambda () (rh-b)))"),
    run("(set 'rh-h '(lambda () (rh-c)))"),
    run("(set 'rh-h '(lambda))"),
    run("(set 'rh-h 5)"),
    run("(set 'rh-h \"str\")"),
    run("(set 'rh-h 'rh-no-such-function)"),
    run("(set 'rh-h (vector 'rh-a))"),
    # A list of functions, called in order with no arguments.
    run("(set 'rh-h (list 'rh-a 'rh-b 'rh-c))"),
    run("(set 'rh-h (list 'rh-a (lambda () (rh-c)) '(lambda () (rh-b))))"),
    run("(set 'rh-h (list '(lambda () (rh-a)) 'rh-b))"),
    run("(set 'rh-h (list (lambda (&optional x) " + rec("(list 'x x)") + ")))"),
    run("(set 'rh-h (list (lambda (x) " + rec("x") + ")))"),
    run("(set 'rh-h (list 'rh-a 5 'rh-b))"),
    run("(set 'rh-h (list 'rh-a 'rh-no-such-function 'rh-b))"),
    run("(set 'rh-h (list 'rh-a nil 'rh-b))"),
    run("(set 'rh-h (list 'rh-a \"s\" 'rh-b))"),
    run("(set 'rh-h (list 'rh-a 'rh-err 'rh-b))"),
    run("(set 'rh-h (cons 'rh-a 'rh-b))"),
    run("(set 'rh-h (cons 'rh-a (cons 'rh-c 5)))"),
    run("(set 'rh-h (list 'rh-a 'rh-a))"),
    # `t' in a value that is not buffer-local: the global value is the same
    # list, walked with its own `t's skipped.
    run("(set 'rh-h (list t))"),
    run("(set 'rh-h (list 'rh-a t 'rh-b))"),
    run("(set 'rh-h (list t t 'rh-c))"),
    # Buffer-local values with `t' (what add-hook LOCAL builds).
    local("(set 'rh-h (list 'rh-c))", "(list 'rh-a t 'rh-b)"),
    local("(set 'rh-h (list 'rh-c))", "(list 'rh-a 'rh-b)"),
    local("(set 'rh-h (list 'rh-c))", "(list t)"),
    local("(set 'rh-h (list 'rh-c))", "(list t t)"),
    local("(set 'rh-h (list 'rh-c))", "(list 'rh-a t)"),
    local("(set 'rh-h nil)", "(list 'rh-a t 'rh-b)"),
    local("", "(list 'rh-a t 'rh-b)", var="rh-void-global"),
    local("", "(list 'rh-a 'rh-b)", var="rh-void-global2"),
    local("(set 'rh-h 'rh-c)", "(list 'rh-a t 'rh-b)"),
    local("(set 'rh-h '(lambda () (rh-c)))", "(list 'rh-a t 'rh-b)"),
    local("(set 'rh-h (lambda () (rh-c)))", "(list 'rh-a t 'rh-b)"),
    local("(set 'rh-h 5)", "(list 'rh-a t 'rh-b)"),
    local("(set 'rh-h (list t 'rh-c t 'rh-a))", "(list t 'rh-b)"),
    local("(set 'rh-h (cons 'rh-c 'rh-a))", "(list t 'rh-b)"),
    local("(set 'rh-h (list 'rh-c 7 'rh-a))", "(list t 'rh-b)"),
    local("(set 'rh-h (list 'rh-c 'rh-err 'rh-a))", "(list t 'rh-b)"),
    local("(set 'rh-h (list (lambda () (rh-c)) 'rh-a))", "(list t 'rh-b)"),
    local("(set 'rh-h (list '(lambda () (rh-c)) 'rh-a))", "(list t 'rh-b)"),
    local("(set 'rh-h (list 'rh-c))", "(cons 'rh-a t)"),
    local("(set 'rh-h (list 'rh-c))", "'rh-a"),
    local("(set 'rh-h (list 'rh-c))", "(list 'rh-a t 5)"),
    # The list may change while it runs: GNU reads the cdr after each call.
    run("(set 'rh-h (list 'rh-m 'rh-a)) (defalias 'rh-m (lambda () " + rec("'m")
        + " (setcdr (symbol-value 'rh-h) (list 'rh-b))))"),
    run("(set 'rh-h (list 'rh-a 'rh-m 'rh-c)) (defalias 'rh-m (lambda () " + rec("'m")
        + " (setcdr (cdr (symbol-value 'rh-h)) nil)))"),
    run("(set 'rh-h (list 'rh-m 'rh-a 'rh-b)) (defalias 'rh-m (lambda () " + rec("'m")
        + " (set 'rh-h nil)))"),
    run("(set 'rh-h (list 'rh-m 'rh-a)) (defalias 'rh-m (lambda () " + rec("'m")
        + " (set 'rh-h (list 'rh-c))))"),
    local("(set 'rh-h (list 'rh-m 'rh-a)) (defalias 'rh-m (lambda () " + rec("'m")
          + " (setcdr (default-value 'rh-h) (list 'rh-c))))", "(list t 'rh-b)"),
    local("(set 'rh-h (list 'rh-c)) (defalias 'rh-m (lambda () " + rec("'m")
          + " (setcdr (symbol-value 'rh-h) (list t 'rh-a))))", "(list 'rh-m 'rh-b)"),
    # Several hooks, in order; one that signals stops the rest.
    run("(set 'rh-h1 (list 'rh-a)) (set 'rh-h3 'rh-c)",
        "(run-hooks 'rh-h1 'rh-unbound-h2 'rh-h3 'rh-h1)"),
    run("(set 'rh-h1 (list 'rh-a)) (set 'rh-h2 nil) (set 'rh-h3 (list 'rh-b 'rh-c))",
        "(run-hooks 'rh-h1 'rh-h2 'rh-h3)"),
    run("(set 'rh-h1 (list 'rh-a 'rh-err 'rh-b)) (set 'rh-h2 'rh-c)",
        "(run-hooks 'rh-h1 'rh-h2)"),
    run("(set 'rh-h1 (list 'rh-a)) (set 'rh-h2 'rh-c)", "(run-hooks 'rh-h1 5 'rh-h2)"),
    run("(set 'rh-h1 (list 'rh-a)) (set 'rh-h2 (list 'rh-b 9))",
        "(run-hooks 'rh-h1 'rh-h2 'rh-h1)"),
    run("(set 'rh-h1 (list 'rh-a)) (set 'rh-h2 'rh-c)",
        "(apply #'run-hooks '(rh-h1 rh-h2 rh-h1))"),
    run("(set 'rh-h1 (list 'rh-a)) (set 'rh-h2 'rh-c)",
        "(funcall #'run-hooks 'rh-h2 'rh-h1)"),
    run("(set 'rh-h1 (list 'rh-a 'rh-b))",
        "(let ((r nil) (i 0)) (while (< i 3) (setq r (cons (run-hooks 'rh-h1) r)) (setq i (1+ i))) r)"),
    # A later hook symbol sees what an earlier hook set.
    run("(set 'rh-h1 (list (lambda () (rh-a) (set 'rh-h2 (list 'rh-b)))))",
        "(run-hooks 'rh-h1 'rh-h2)"),
    # Recursion, non-local exits, return value.
    run("(set 'rh-h (list 'rh-a 'rh-r 'rh-c)) (set 'rh-inner (list 'rh-b 'rh-b))"
        " (defalias 'rh-r (lambda () " + rec("'r") + " (run-hooks 'rh-inner)))"),
    run("(set 'rh-h (list 'rh-a 'rh-r 'rh-c)) (defalias 'rh-r (lambda () "
        + rec("(list 'depth (length (symbol-value 'rh-log)))")
        + " (if (< (length (symbol-value 'rh-log)) 6) (run-hooks 'rh-h))))"),
    run("(set 'rh-h (list 'rh-a (lambda () (throw 'rh-tag 'thrown)) 'rh-b))",
        "(catch 'rh-tag (run-hooks 'rh-h))"),
    run("(set 'rh-h (list 'rh-a (lambda () (signal 'wrong-type-argument (list 'x))) 'rh-b))"),
    run("(set 'rh-h (list 'rh-a 'rh-b))", "(list (run-hooks 'rh-h) (run-hooks 'rh-h 'rh-h))"),
    # Aliases and dynamic bindings are resolved like `symbol-value'.
    run("(set 'rh-h (list 'rh-a)) (defvaralias 'rh-alias 'rh-h)", "(run-hooks 'rh-alias)"),
    run("(set 'rh-h (list 'rh-a t 'rh-b)) (defvaralias 'rh-alias2 'rh-h)", "(run-hooks 'rh-alias2)"),
    run("(defvar rh-dyn nil) (set 'rh-dyn (list 'rh-c))",
        "(let ((rh-dyn (list 'rh-a t))) (run-hooks 'rh-dyn))"),
    # GC pressure inside hook functions: the local tail, the global tail and
    # the remaining hook arguments are held across each callback.
    run(f"(defalias 'rh-gc (lambda () {GC} {REC['gc']})) (set 'rh-h1 (list 'rh-gc 'rh-a 'rh-gc 'rh-b)) (set 'rh-h2 (list 'rh-c))",
        "(run-hooks 'rh-h1 'rh-h2 'rh-h1)"),
    local(f"(defalias 'rh-gc (lambda () {GC} {REC['gc']})) (set 'rh-h (list 'rh-gc 'rh-a t 'rh-gc 'rh-c))",
          "(list 'rh-gc t 'rh-b 'rh-gc)"),
    run("(set 'rh-h (list 'rh-a))",
        f"(let* ((x (list 1 (make-string 2 ?z))) (f (lambda () {GC} (set 'rh-log (cons x (symbol-value 'rh-log)))))"
        " (h (list f f))) (set 'rh-h h) (run-hooks 'rh-h)"
        " (list (eq (symbol-value 'rh-h) h) (eq (car (symbol-value 'rh-log)) x)"
        " (eq (car (cdr (symbol-value 'rh-log))) x) (equal x (list 1 \"zz\"))))"),
    run(f"(defalias 'rh-gc (lambda () {GC} {REC['gc']}))",
        "(let ((s (make-symbol \"rh-uninterned\")) (s2 (make-symbol \"rh-u2\")))"
        " (set s (list 'rh-gc 'rh-a)) (set s2 (list 'rh-gc 'rh-b)) (run-hooks s s2 s))"),
    run(f"(set 'rh-h (list (lambda () {GC} (rh-a)) (lambda () {GC} (rh-b))))",
        "(run-hooks 'rh-h 'rh-h 'rh-h 'rh-h)"),
]

p = Path(sys.argv[1])
spec = json.loads(p.read_text())
spec["probes"] = PROBES
p.write_text(json.dumps(spec) + "\n")
print(f"{p}: {len(PROBES)} probes")
