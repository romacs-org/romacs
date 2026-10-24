#!/usr/bin/env python3
"""Augment the from-manifest specs of the command-loop primitives with probes.

Inputs only: every answer is recorded from live GNU 32.0.50 (fresh process
per case, byte-compiled and interpreted).  Probes print no function objects
(GNU and VM2 print closures differently), so a difference is about behaviour.

  python3 make_spec.py SPECDIR
"""
import json, sys
from pathlib import Path

LOG = "(set 'cl-log nil)"
def rec(x):
    return f"(set 'cl-log (cons {x} (symbol-value 'cl-log)))"
def logged(form):
    return f"(progn {LOG} (list (condition-case e {form} (error (list 'err e))) (nreverse (symbol-value 'cl-log))))"
GC = "(let ((i 0)) (while (< i 3000) (list i (make-string 3 ?x) (vector i)) (setq i (1+ i))))"
ARGS = "(lambda (&rest a) " + rec("a") + " a)"

RUN_HOOK_WITH_ARGS = [
    "(run-hook-with-args 'cl-never-bound-hook 1 2)",
    "(run-hook-with-args nil 1)",
    "(run-hook-with-args t 1)",
    "(run-hook-with-args 5 1)",
    "(run-hook-with-args \"h\")",
    "(run-hook-with-args :cl-key 1)",
    "(run-hook-with-args '(cl-h) 1)",
    logged("(progn (set 'cl-h nil) (run-hook-with-args 'cl-h 1))"),
    # A value that is one function is called once with the arguments.
    logged(f"(progn (set 'cl-h {ARGS}) (run-hook-with-args 'cl-h 1 2))"),
    logged(f"(progn (defalias 'cl-f {ARGS}) (set 'cl-h 'cl-f) (run-hook-with-args 'cl-h 'x \"y\" 3.5))"),
    logged("(progn (set 'cl-h 'cl-no-such-function) (run-hook-with-args 'cl-h 1))"),
    logged("(progn (set 'cl-h 5) (run-hook-with-args 'cl-h 1))"),
    logged("(progn (set 'cl-h (lambda (a b) (list a b))) (run-hook-with-args 'cl-h 1 2))"),
    logged("(progn (set 'cl-h (lambda (a) " + rec("a") + ")) (run-hook-with-args 'cl-h 1 2))"),
    logged("(progn (set 'cl-h (lambda (a b c) " + rec("a") + ")) (run-hook-with-args 'cl-h 1 2))"),
    logged("(progn (set 'cl-h (lambda (a &optional b) " + rec("(list a b)") + ")) (run-hook-with-args 'cl-h 1))"),
    logged("(progn (set 'cl-h (lambda () " + rec("'none") + ")) (run-hook-with-args 'cl-h))"),
    # Lists of functions, each called with the same arguments, in order.
    logged(f"(progn (set 'cl-h (list {ARGS} {ARGS})) (run-hook-with-args 'cl-h 'a 'b))"),
    logged(f"(progn (defalias 'cl-f {ARGS}) (set 'cl-h (list 'cl-f (lambda (x) " + rec("(list 'second x)") + "))) (run-hook-with-args 'cl-h 7))"),
    logged(f"(progn (set 'cl-h (list {ARGS} 'cl-no-such-function {ARGS})) (run-hook-with-args 'cl-h 1))"),
    logged(f"(progn (set 'cl-h (list {ARGS} 5 {ARGS})) (run-hook-with-args 'cl-h 1))"),
    logged(f"(progn (set 'cl-h (list {ARGS} nil {ARGS})) (run-hook-with-args 'cl-h 1))"),
    logged(f"(progn (set 'cl-h (list {ARGS} (lambda (x) (error \"cl-boom %s\" x)) {ARGS})) (run-hook-with-args 'cl-h 1))"),
    logged(f"(progn (set 'cl-h (cons {ARGS} {ARGS})) (run-hook-with-args 'cl-h 1))"),
    logged(f"(progn (set 'cl-h (cons {ARGS} 5)) (run-hook-with-args 'cl-h 1))"),
    logged(f"(progn (set 'cl-h (list '(lambda (x) " + rec("(list 'lam x)") + "))) (run-hook-with-args 'cl-h 1))"),
    logged(f"(progn (set 'cl-h '(lambda (x) " + rec("(list 'lam x)") + ")) (run-hook-with-args 'cl-h 1))"),
    # Return values of hook functions are ignored: the result is nil.
    "(progn (set 'cl-h (list (lambda (x) (list 'ret x)) (lambda (x) t))) (run-hook-with-args 'cl-h 1))",
    "(progn (set 'cl-h (lambda (x) (list 'ret x))) (run-hook-with-args 'cl-h 1))",
    # `t' in a non-local value: the global value is the same list; its `t's are skipped.
    logged(f"(progn (set 'cl-h (list {ARGS} t {ARGS})) (run-hook-with-args 'cl-h 1))"),
    logged(f"(progn (set 'cl-h (list t t {ARGS})) (run-hook-with-args 'cl-h 1))"),
    # Buffer-local values with `t' (add-hook LOCAL builds these).
    *[logged("(with-temp-buffer (set 'cl-h " + g + ") (make-local-variable 'cl-h) (set 'cl-h " + l
             + ") (run-hook-with-args 'cl-h 'arg 2))") for g, l in [
        ("(list (lambda (&rest a) " + rec("(cons 'global a)") + "))", f"(list {ARGS} t {ARGS})"),
        ("(list (lambda (&rest a) " + rec("(cons 'global a)") + "))", f"(list {ARGS} {ARGS})"),
        ("(list (lambda (&rest a) " + rec("(cons 'global a)") + "))", "(list t)"),
        ("nil", f"(list {ARGS} t)"),
        ("(lambda (&rest a) " + rec("(cons 'whole a)") + ")", f"(list t {ARGS})"),
        ("'(lambda (&rest a) " + rec("(cons 'lam a)") + ")", f"(list t {ARGS})"),
        ("5", f"(list t {ARGS})"),
        ("(list t (lambda (&rest a) " + rec("(cons 'g1 a)") + ") t (lambda (&rest a) " + rec("(cons 'g2 a)") + "))", "(list t)"),
        ("(cons (lambda (&rest a) " + rec("(cons 'g1 a)") + ") 7)", f"(list t {ARGS})"),
        ("(list (lambda (&rest a) " + rec("(cons 'g a)") + "))", f"(cons {ARGS} t)"),
        ("(list (lambda (&rest a) " + rec("(cons 'g a)") + "))", f"{ARGS}"),
    ]],
    logged(f"(with-temp-buffer (make-local-variable 'cl-void-global) (set 'cl-void-global (list {ARGS} t {ARGS})) (run-hook-with-args 'cl-void-global 1))"),
    # The list may change while it runs: GNU reads the cdr after each call.
    logged("(progn (defalias 'cl-m (lambda (x) " + rec("(list 'm x)") + " (setcdr (symbol-value 'cl-h) (list (lambda (y) " + rec("(list 'added y)") + "))))) (set 'cl-h (list 'cl-m (lambda (y) " + rec("(list 'dropped y)") + "))) (run-hook-with-args 'cl-h 1))"),
    logged("(progn (defalias 'cl-m (lambda (x) " + rec("(list 'm x)") + " (set 'cl-h nil))) (set 'cl-h (list 'cl-m (lambda (y) " + rec("(list 'still y)") + "))) (run-hook-with-args 'cl-h 1))"),
    logged("(with-temp-buffer (set 'cl-h (list 'cl-m (lambda (y) " + rec("(list 'gtail y)") + "))) (defalias 'cl-m (lambda (x) " + rec("(list 'm x)") + " (setcdr (default-value 'cl-h) (list (lambda (y) " + rec("(list 'gadded y)") + "))))) (make-local-variable 'cl-h) (set 'cl-h (list t)) (run-hook-with-args 'cl-h 1))"),
    # Arguments are passed as the same objects, unchanged by earlier functions.
    logged("(let ((x (list 1 2))) (set 'cl-h (list (lambda (a) (setcar a 'changed)) (lambda (a) " + rec("(eq a x)") + "))) (run-hook-with-args 'cl-h x) x)"),
    # Aliases and dynamic bindings resolve like symbol-value.
    logged(f"(progn (set 'cl-h (list {ARGS})) (defvaralias 'cl-alias 'cl-h) (run-hook-with-args 'cl-alias 1))"),
    logged(f"(progn (defvar cl-dyn nil) (let ((cl-dyn (list {ARGS}))) (run-hook-with-args 'cl-dyn 'dyn)))"),
    # Recursion, throw.
    logged("(progn (set 'cl-h (list (lambda (n) " + rec("n") + " (if (< n 3) (run-hook-with-args 'cl-h (1+ n)))))) (run-hook-with-args 'cl-h 0))"),
    logged("(catch 'cl-tag (set 'cl-h (list (lambda (x) " + rec("x") + " (throw 'cl-tag 'thrown)) (lambda (x) " + rec("'after") + "))) (run-hook-with-args 'cl-h 1))"),
    "(list (run-hook-with-args 'cl-unbound) (funcall #'run-hook-with-args 'cl-unbound 1) (apply #'run-hook-with-args '(cl-unbound 1 2)))",
    # GC pressure inside hook functions: the arguments and tails are held across.
    logged(f"(let ((x (list 1 (make-string 2 ?z)))) (set 'cl-h (list (lambda (a) {GC} " + rec("(eq a x)") + ") t (lambda (a) {GC} " + rec("(equal a (list 1 \"zz\"))") + "))) (with-temp-buffer (make-local-variable 'cl-h) (set 'cl-h (list (lambda (a) {GC} " + rec("(eq a x)") + ") t)) (run-hook-with-args 'cl-h x)))"),
]

PREFIX = ["(prefix-numeric-value '-)", "(prefix-numeric-value '(4))", "(prefix-numeric-value '(-16))",
          "(prefix-numeric-value '(foo))", "(prefix-numeric-value '(4 . 5))", "(prefix-numeric-value '(1.5))",
          "(prefix-numeric-value 1.5)", "(prefix-numeric-value -7)", "(prefix-numeric-value 'foo)",
          "(prefix-numeric-value '--)", "(prefix-numeric-value \"-\")", "(prefix-numeric-value (list most-negative-fixnum))",
          "(let ((x (list 4))) (eq (prefix-numeric-value x) (car x)))"]

FUNCALL_INTERACTIVELY = [
    "(funcall-interactively 'list 1 2)",
    "(funcall-interactively #'list)",
    "(funcall-interactively 'car)",
    "(funcall-interactively 'car '(1 2))",
    "(funcall-interactively 'cl-no-such-fn)",
    "(funcall-interactively 5)",
    "(funcall-interactively nil)",
    "(funcall-interactively (lambda (&rest a) (reverse a)) 1 2 3)",
    "(funcall-interactively (lambda (a) a))",
    "(funcall-interactively (lambda () (error \"cl-boom\")))",
    "(funcall-interactively 'funcall-interactively 'list 'nested)",
    "(apply #'funcall-interactively 'list '(1 2))",
    "(catch 'cl-tag (funcall-interactively (lambda () (throw 'cl-tag 'thrown))))",
    "(let ((x (list 1))) (eq (funcall-interactively 'identity x) x))",
    "(funcall-interactively '(lambda (a) (list 'lam a)) 1)",
    "(funcall-interactively (lambda () (called-interactively-p 'any)))",
    f"(funcall-interactively (lambda (a) {GC} a) (list 'kept))",
]

IFORM = [
    "(interactive-form (lambda (x) (interactive \"p\") x))",
    "(interactive-form (lambda (x) \"doc\" (interactive \"p\") x))",
    "(interactive-form (lambda (x) (interactive) x))",
    "(interactive-form (lambda () (interactive)))",
    "(interactive-form (lambda (x) (interactive (list 1)) x))",
    "(interactive-form (lambda (x) \"doc\"))",
    "(interactive-form (lambda (x) \"doc\" x))",
    "(interactive-form (lambda (x) (interactive \"p\" cl-mode) x))",
    "(interactive-form (lambda (x) (interactive \"p\" cl-mode cl-mode2) x))",
    "(interactive-form (lambda (x) x (interactive \"p\")))",
    "(interactive-form (lambda (x) \"doc\" x (interactive \"p\")))",
    "(interactive-form (lambda (x) \"doc\" \"more\" (interactive \"p\") x))",
    "(interactive-form (lambda (x) (interactive \"P\") (interactive \"p\") x))",
    "(interactive-form (lambda (x) (interactive \"\") x))",
    "(interactive-form (lambda (x) (interactive 'cl-sym) x))",
    "(let ((x 5)) (interactive-form (lambda (a) (interactive (list x)) a)))",
    "(let ((f (lambda () (interactive \"p\") 1))) (eq (interactive-form f) (interactive-form f)))",
    "(let ((f (lambda () (interactive \"p\") 1))) (eq (car (cdr (interactive-form f))) (car (cdr (interactive-form f)))))",
    "(funcall (lambda () (interactive \"p\") 42))",
    "(funcall (lambda () \"doc\" (interactive \"p\")))",
    "(funcall (lambda () (interactive \"p\")))",
    "(funcall (lambda (x) (interactive \"p\") (list 'body x)) 3)",
    "(interactive-form '(lambda (x) (interactive \"p\") x))",
    "(interactive-form '(lambda (x) \"d\" (interactive \"p\") x))",
    "(interactive-form '(lambda (x) x (interactive \"p\")))",
    "(interactive-form '(lambda (x) (interactive \"p\" m1 m2) x))",
    "(interactive-form '(lambda (x) (interactive) x))",
    "(interactive-form '(lambda (x) x))",
    "(interactive-form '(lambda))",
    "(interactive-form '(lambda . 5))",
    "(interactive-form '(lambda x . 5))",
    "(interactive-form '(lambda (x) 5 (interactive \"p\") . 6))",
    "(interactive-form '(lambda (x) (interactive . 5)))",
    "(interactive-form '(lambda (x) (interactive \"p\" . 5)))",
    "(interactive-form '(closure (t) (x) (interactive \"p\") x))",
    "(interactive-form '(foo (x) (interactive \"p\") x))",
    *[f"(interactive-form '{n})" for n in [
        "forward-char", "backward-char", "beginning-of-line", "end-of-line", "forward-line",
        "delete-char", "self-insert-command", "goto-char", "delete-region", "narrow-to-region",
        "widen", "erase-buffer", "search-forward", "re-search-forward", "re-search-backward",
        "modify-syntax-entry", "make-variable-buffer-local", "make-local-variable", "car",
        "insert", "point", "set-buffer", "current-buffer", "interactive-form", "call-interactively",
        "funcall-interactively", "run-hooks"]],
    "(interactive-form 'self-insert-command)",
    "(let ((a (interactive-form 'self-insert-command)) (b (interactive-form 'self-insert-command))) (list (equal a b) (eq a b) (eq (car (cdr a)) (car (cdr b)))))",
    "(interactive-form (symbol-function 'forward-char))",
    "(interactive-form nil)",
    "(interactive-form t)",
    "(interactive-form 5)",
    "(interactive-form \"forward-char\")",
    "(interactive-form 'cl-no-such-fn)",
    "(interactive-form [interactive \"p\"])",
    "(interactive-form '(interactive \"p\"))",
    # Symbols: the `interactive-form' property on any symbol of the alias chain wins.
    "(progn (defalias 'cl-a (lambda () (interactive \"P\") 1)) (put 'cl-a 'interactive-form '(interactive \"p\")) (interactive-form 'cl-a))",
    "(progn (defalias 'cl-a (lambda () (interactive \"P\") 1)) (put 'cl-a 'interactive-form '(interactive \"p\")) (defalias 'cl-b 'cl-a) (interactive-form 'cl-b))",
    "(progn (defalias 'cl-c (lambda () (interactive \"P\") 1)) (defalias 'cl-d 'cl-c) (put 'cl-d 'interactive-form '(interactive \"x\")) (list (interactive-form 'cl-d) (interactive-form 'cl-c)))",
    "(progn (put 'cl-e 'interactive-form '(interactive \"s\")) (interactive-form 'cl-e))",
    "(progn (defalias 'cl-f2 'car) (put 'cl-f2 'interactive-form 'not-a-list) (interactive-form 'cl-f2))",
    "(progn (defalias 'cl-g (lambda () 1)) (put 'cl-g 'interactive-form '(interactive \"p\")) (interactive-form 'cl-g))",
    "(progn (defalias 'cl-h1 'forward-char) (defalias 'cl-h2 'cl-h1) (interactive-form 'cl-h2))",
    "(progn (defalias 'cl-i (lambda (x) (interactive \"p\") x)) (interactive-form 'cl-i))",
    "(progn (defalias 'cl-j '(lambda (x) (interactive \"P\") x)) (interactive-form 'cl-j))",
    "(progn (fset 'cl-k (lambda (x) (interactive \"p\") x)) (fset 'cl-k (lambda (x) x)) (interactive-form 'cl-k))",
    # defun: docstring, declare, interactive (byte-run.el's defun).
    "(progn (defun cl-d1 (x) \"Doc.\" (interactive \"p\") x) (interactive-form 'cl-d1))",
    "(progn (defun cl-d2 (x) \"Doc.\" (declare (interactive-only t)) (interactive \"p\") x) (interactive-form 'cl-d2))",
    "(progn (defun cl-d3 (x) (declare (indent 1)) (interactive \"P\") x) (interactive-form 'cl-d3))",
    "(progn (defun cl-d4 () \"Doc.\") (interactive-form 'cl-d4))",
    "(progn (defun cl-d5 () (interactive)) (list (interactive-form 'cl-d5) (cl-d5)))",
    "(progn (defun cl-d6 () \"Doc.\" (interactive) 9) (list (interactive-form 'cl-d6) (cl-d6)))",
    "(progn (defun cl-d7 (n) (interactive (list (* 2 (prefix-numeric-value current-prefix-arg)))) n) (interactive-form 'cl-d7))",
    "(progn (defun cl-d8 () (interactive \"p\" cl-mode)) (interactive-form 'cl-d8))",
    "(progn (defmacro cl-mac (x) (interactive \"p\") x) (interactive-form 'cl-mac))",
    # Autoload: GNU loads the file first (Fautoload_do_load).
    "(progn (autoload 'cl-al \"cl-no-such-file-anywhere\" nil t) (interactive-form 'cl-al))",
    "(progn (autoload 'cl-al2 \"cl-no-such-file-anywhere\" nil nil) (interactive-form 'cl-al2))",
    # Advice combines specs (nadvice.el).
    "(progn (defalias 'cl-adv (lambda (x) (interactive \"p\") x)) (advice-add 'cl-adv :around (lambda (f &rest a) (apply f a))) (interactive-form 'cl-adv))",
    f"(let ((f (lambda () (interactive (list 'kept)) 1))) {GC} (list (interactive-form f) (call-interactively f)))",
    # Review round 1 (Sol): (:documentation FORM), dotted interactive forms.
    "(funcall (lambda () (:documentation \"d\")))",
    "(interactive-form (lambda () (:documentation \"d\") \"s\" (interactive \"p\") 1))",
    "(interactive-form (lambda () (:documentation \"d\") (interactive \"p\") 1))",
    "(eval '(condition-case e (lambda () (:documentation . 5) 1) (error e)) t)",
    "(eval '(interactive-form (function (lambda () (interactive \"p\" . 3) 9))) t)",
    "(eval '(condition-case e (interactive-form (function (lambda () (interactive . 3) 9))) (error e)) t)",
    "(progn (defun cl-q () (:documentation \"x\") (interactive \"p\") 1) (interactive-form 'cl-q))",
    "(eval '(let ((x 0)) (let ((f (function (lambda () (:documentation (setq x 5)) 9)))) (list x (funcall f) x))) t)",
    # Review round 2 (Sol): a dotted (:documentation FORM . X) still evaluates
    # FORM; `defun' takes its metadata forms in any order (byte-run.el:324-356).
    "(eval '(let ((x 0)) (let ((f (function (lambda () (:documentation (setq x 5) . 3) 9)))) (list x (funcall f) x))) t)",
    "(eval '(funcall (function (lambda () (:documentation) 9))) t)",
    "(progn (defun cl-m1 () (declare (indent 1)) (:documentation \"x\") (interactive) 1) (list (interactive-form 'cl-m1) (cl-m1)))",
    "(progn (defun cl-m2 () \"doc\" (:documentation \"x\") (interactive) 1) (list (interactive-form 'cl-m2) (cl-m2)))",
    "(progn (defun cl-m3 () (interactive \"p\") \"Doc.\" 1) (list (interactive-form 'cl-m3) (cl-m3)))",
    "(progn (defun cl-m4 () (interactive \"P\") (declare (indent 1)) 2) (list (interactive-form 'cl-m4) (cl-m4)))",
    "(progn (defun cl-m5 () (interactive \"p\") (interactive \"P\") 3) (list (interactive-form 'cl-m5) (cl-m5)))",
    "(progn (defun cl-m7 () (declare (indent 1)) \"doc\" (interactive) 4) (list (interactive-form 'cl-m7) (cl-m7)))",
    "(progn (defun cl-m8 () \"Doc.\" (declare (indent 1))) (list (interactive-form 'cl-m8) (cl-m8)))",
    "(progn (defun cl-m9 () (declare (indent 1)) (declare (doc-string 1)) (interactive \"p\") 5) (list (interactive-form 'cl-m9) (cl-m9)))",
    "(progn (defun cl-m10 () (interactive \"p\") (:documentation \"x\") 6) (list (interactive-form 'cl-m10) (cl-m10)))",
    "(progn (defun cl-m11 () (:documentation \"x\") (declare (indent 1)) (interactive \"P\") 7) (list (interactive-form 'cl-m11) (cl-m11)))",
    "(eval '(let ((x 0)) (defun cl-m12 () (declare (indent 1)) (:documentation (setq x 5)) (interactive) 8) (list x (cl-m12) x)) t)",
    # A dropped metadata form after a later `interactive' (mutant if-defun-order-fixed).
    "(progn (defun cl-m13 () (:documentation \"x\") (interactive \"p\") (:documentation \"y\") 3) (list (interactive-form 'cl-m13) (cl-m13)))",
    "(progn (defun cl-m14 () (declare (indent 1)) (interactive \"P\") (declare (doc-string 1)) 9) (list (interactive-form 'cl-m14) (cl-m14)))",
    # ... and only an effectful dropped doc form shows it (review round 4, Sol).
    "(eval '(let ((x 0)) (defun cl-m15 () (:documentation \"x\") (interactive \"p\") (:documentation (setq x 5)) 3) (list x (cl-m15) x (interactive-form 'cl-m15))) t)",
]

def ci(body, spec):
    return f"(call-interactively (lambda (&rest a) (interactive {spec}) {body}))"

CALL_INTERACTIVELY = [
    "(call-interactively (lambda (a) (interactive \"p\") a))",
    "(call-interactively (lambda (a) (interactive \"P\") a))",
    "(let ((current-prefix-arg '(4))) (call-interactively (lambda (a b) (interactive \"p\\nP\") (list a b))))",
    "(let ((current-prefix-arg 3)) (call-interactively (lambda (a b) (interactive \"p\\nP\") (list a b))))",
    "(let ((current-prefix-arg '-)) (call-interactively (lambda (a b) (interactive \"p\\nP\") (list a b))))",
    "(let ((current-prefix-arg 'foo)) (call-interactively (lambda (a b) (interactive \"p\\nP\") (list a b))))",
    "(let ((prefix-arg 3)) (call-interactively (lambda (a) (interactive \"P\") a)))",
    "(let ((prefix-arg 3)) (call-interactively (lambda (a) (interactive \"p\") a)))",
    "(let ((current-prefix-arg 5)) (call-interactively (lambda (a) (interactive \"P\") (setq current-prefix-arg 9) a)))",
    "(let ((current-prefix-arg 5)) (list (call-interactively (lambda (a) (interactive \"p\") (setq current-prefix-arg 9) a)) current-prefix-arg))",
    # this-command & co. are saved at entry and restored before the call.
    "(let ((this-command 'tc) (real-this-command 'rtc) (this-original-command 'toc) (last-command 'lc)) (list (call-interactively (lambda () (interactive) (list this-command real-this-command this-original-command last-command))) this-command))",
    "(let ((this-command 'tc) (real-this-command 'rtc)) (list (call-interactively (lambda () (interactive) (setq this-command 'x real-this-command 'y) 1)) this-command real-this-command))",
    "(let ((this-command 'tc)) (list (call-interactively (lambda () (interactive (progn (setq this-command 'z) nil)) this-command)) this-command))",
    "(let ((last-command 'lc)) (list (call-interactively (lambda () (interactive (progn (setq last-command 'z) nil)) last-command)) last-command))",
    "(let ((this-command 'tc)) (call-interactively (lambda (x) (interactive (list this-command)) x)))",
    # Non-commands.
    "(call-interactively (lambda () 1))",
    "(call-interactively 'car)",
    "(call-interactively 'cl-no-such-fn)",
    "(call-interactively nil)",
    "(call-interactively 5)",
    "(call-interactively \"forward-char\")",
    "(call-interactively '(lambda (x) x))",
    "(call-interactively '(lambda (&rest a) (interactive \"p\") a))",
    "(call-interactively '(lambda (&rest a) \"doc\" (ignore) (interactive (list 1 2)) a))",
    "(progn (defalias 'cl-a (lambda (&rest a) (interactive \"P\") a)) (call-interactively 'cl-a))",
    "(progn (defalias 'cl-a (lambda (&rest a) (interactive \"P\") a)) (defalias 'cl-b 'cl-a) (let ((current-prefix-arg '(16))) (call-interactively 'cl-b)))",
    "(progn (defalias 'cl-a (lambda (&rest a) (interactive \"P\") a)) (put 'cl-a 'interactive-form '(interactive \"p\")) (call-interactively 'cl-a))",
    "(progn (defalias 'cl-a (lambda (&rest a) a)) (put 'cl-a 'interactive-form '(interactive (list 'from-prop))) (call-interactively 'cl-a))",
    "(progn (defalias 'cl-a 'list) (put 'cl-a 'interactive-form '(interactive \"i\\np\")) (call-interactively 'cl-a))",
    "(progn (defalias 'interactive-form (lambda (f) '(interactive (list 'advised)))) (call-interactively 'list))",
    "(progn (defalias 'funcall-interactively (lambda (f &rest a) (list 'fi f a))) (call-interactively (lambda (&rest a) (interactive \"p\") a)))",
    # Spec codes.
    ci("a", "\"i\\ni\""), ci("a", "\"\""), ci("a", "nil"), ci("a", "\"p\\n\""), ci("a", "\"p\\n\\np\""),
    ci("a", "\"pfoo\\nPbar\""), ci("a", "\"*p\""), ci("a", "\"-p\""), ci("a", "\"+p\""), ci("a", "\"q\""),
    ci("a", "\"pq\\nλ\""), ci("a", "\"*-*p\""), ci("a", "\"^p\""), ci("a", "\"@p\""), ci("a", "\"^@*-i\""),
    ci("a", "\"U\""), ci("a", "\"e\""), ci("a", "\"Z\""), ci("a", "\"N\""), ci("a", "\"m\""), ci("a", "\"r\""),
    ci("a", "\"d\""),
    "(let ((current-prefix-arg '(4))) " + ci("a", "\"N\"") + ")",
    "(let ((current-prefix-arg '(4))) " + ci("a", "\"Z\"") + ")",
    "(with-temp-buffer (insert \"abc\") (goto-char 2) " + ci("a", "\"d\\nd\"") + ")",
    "(with-temp-buffer (insert \"αβγ\") (goto-char 3) " + ci("a", "\"d\"") + ")",
    "(with-temp-buffer (insert \"abc\") (goto-char 2) " + ci("a", "\"r\"") + ")",
    "(with-temp-buffer (insert \"abcdef\") (goto-char 2) (set-mark 5) " + ci("a", "\"r\"") + ")",
    "(with-temp-buffer (setq buffer-read-only t) " + ci("a", "\"*p\"") + ")",
    "(with-temp-buffer (setq buffer-read-only t) (let ((inhibit-read-only t)) " + ci("a", "\"*p\"") + "))",
    "(call-interactively (lambda (&rest a) (interactive \"e\") a) nil [(mouse-1 (foo))])",
    "(call-interactively (lambda (&rest a) (interactive \"e\\ne\") a) nil [(m1 1) x (m2 2)])",
    "(call-interactively (lambda (&rest a) (interactive \"e\\ne\") a) nil [(m1 1)])",
    "(let ((inhibit-mouse-event-check t)) (call-interactively (lambda (&rest a) (interactive \"e\\ne\") a) nil [x (m1 1) y]))",
    "(let ((command-history nil)) (call-interactively (lambda (&rest a) (interactive \"e\\nd\") a) t [(m1 sym)]) (cdr (car command-history)))",
    "(let ((command-history nil)) (call-interactively (lambda (&rest a) (interactive (list 1 2)) a) t) (call-interactively (lambda (&rest a) (interactive (list 3)) a) t) (list (length command-history) (cdr (car command-history)) (cdr (car (cdr command-history)))))",
    "(let ((command-history nil) (history-length 2)) (call-interactively (lambda (&rest a) (interactive (list 1)) a) t) (call-interactively (lambda (&rest a) (interactive (list 2)) a) t) (call-interactively (lambda (&rest a) (interactive (list 3)) a) t) (list (length command-history) (cdr (car command-history)) (cdr (car (cdr command-history)))))",
    "(let ((command-history nil)) (put 'command-history 'history-length 1) (call-interactively (lambda (&rest a) (interactive (list 1)) a) t) (call-interactively (lambda (&rest a) (interactive (list 2)) a) t) (list (length command-history) (cdr (car command-history))))",
    "(let ((command-history nil) (history-length 0)) (call-interactively (lambda (&rest a) (interactive (list 1)) a) t) command-history)",
    "(let ((command-history 'not-a-list)) (call-interactively (lambda (&rest a) (interactive (list 1)) a) t) command-history)",
    "(let ((command-history nil)) (defalias 'cl-same (lambda (&rest a) (interactive (list 1)) a)) (call-interactively 'cl-same t) (call-interactively 'cl-same t) command-history)",
    "(call-interactively (lambda (&rest a) (interactive \"p\") a) nil 5)",
    "(call-interactively (lambda (&rest a) (interactive \"p\") a) nil [1 2])",
    "(call-interactively (lambda (&rest a) (interactive \"p\") a) nil \"ab\")",
    "(progn (defalias 'cl-e (lambda (&rest a) (interactive \"e\") a)) (call-interactively 'cl-e))",
    "(progn (defalias 'handle-shift-selection (lambda () " + rec("'shift") + ")) " + logged(ci("a", "\"^p\"")) + ")",
    "(progn (defalias 'handle-shift-selection (lambda () " + rec("'shift") + ")) " + logged(ci("a", "\"p\"")) + ")",
    "(progn (defalias 'use-region-p (lambda () nil)) " + ci("a", "\"R\"") + ")",
    "(progn (defalias 'use-region-p (lambda () t)) " + ci("a", "\"R\"") + ")",
    # The minibuffer/keyboard-reading codes (a b B c C D f F G k K M n N s S
    # v x X z Z) block on input in a batch GNU, so they cannot be recorded
    # here; VM2 signals for them (DIVERGENCES VM2-CALLINT1). A `%' in a prompt
    # needs format-message, which VM2 lacks.
    ci("a", "\"pP%s\""),
    # List specs: evaluated, in the closure's lexical environment.
    ci("a", "(list 1 2)"), ci("a", "7"), ci("a", "'(1 . 2)"), ci("a", "(error \"cl-boom\")"),
    ci("a", "'cl-unbound-variable"), ci("a", "(list current-prefix-arg)"),
    "(let ((x 5)) (call-interactively (lambda (a) (interactive (list x)) a)))",
    "(let ((x 1)) (let ((f (lambda (a) (interactive (list (setq x (1+ x)))) a))) (list (call-interactively f) (call-interactively f) x)))",
    "(let ((f (let ((y 'inner)) (lambda (a) (interactive (list y)) a)))) (call-interactively f))",
    "(progn (defvar cl-dynvar 'global) (let ((cl-dynvar 'let)) (call-interactively (lambda (a) (interactive (list cl-dynvar)) a))))",
    "(progn (defun cl-d7 (n) (interactive (list (* 2 (prefix-numeric-value current-prefix-arg)))) n) (let ((current-prefix-arg 4)) (call-interactively 'cl-d7)))",
    "(progn (defun cl-d9 (n) \"Doc.\" (interactive \"p\") (list 'd9 n)) (let ((current-prefix-arg '(4))) (call-interactively 'cl-d9)))",
    "(call-interactively (lambda (&rest a) (interactive (list (called-interactively-p 'any))) (cons (called-interactively-p 'any) a)))",
    # Subrs and their GNU interactive specs.
    "(with-temp-buffer (insert \"abc\") (goto-char 1) (list (call-interactively 'forward-char) (point)))",
    "(with-temp-buffer (insert \"abc\") (goto-char 1) (let ((current-prefix-arg 2)) (list (call-interactively 'forward-char) (point))))",
    "(with-temp-buffer (insert \"abc\") (goto-char 4) (let ((current-prefix-arg '-)) (list (call-interactively 'forward-char) (point))))",
    "(with-temp-buffer (insert \"abc\") (goto-char 4) (list (condition-case e (call-interactively 'forward-char) (error e)) (point)))",
    "(with-temp-buffer (insert \"abc\") (goto-char 4) (list (call-interactively 'backward-char) (point)))",
    "(with-temp-buffer (insert \"ab\\ncd\") (goto-char 5) (list (call-interactively 'beginning-of-line) (point)))",
    "(with-temp-buffer (insert \"ab\\ncd\") (goto-char 1) (list (call-interactively 'end-of-line) (point)))",
    "(with-temp-buffer (insert \"abc\") (goto-char 1) (list (call-interactively 'delete-char) (buffer-string)))",
    "(with-temp-buffer (insert \"abc\") (goto-char 2) (let ((last-command-event ?z)) (list (call-interactively 'self-insert-command) (buffer-string))))",
    "(with-temp-buffer (insert \"abc\") (goto-char 2) (let ((last-command-event ?z) (current-prefix-arg 3)) (list (call-interactively 'self-insert-command) (buffer-string))))",
    "(with-temp-buffer (insert \"abc\") (narrow-to-region 2 3) (list (call-interactively 'widen) (point-min) (point-max)))",
    "(with-temp-buffer (insert \"abc\") (list (call-interactively 'erase-buffer) (buffer-string)))",
    # command-history with RECORD-FLAG.
    "(let ((command-history nil)) (call-interactively (lambda (&rest a) (interactive (list 1 'sym \"s\" '(l))) a) t) (cdr (car command-history)))",
    "(let ((command-history nil)) (defalias 'cl-h (lambda (&rest a) (interactive \"p\\nP\\ni\") a)) (call-interactively 'cl-h t) command-history)",
    "(let ((command-history nil)) (defalias 'cl-h (lambda (&rest a) (interactive \"p\") a)) (call-interactively 'cl-h nil) command-history)",
    "(let ((command-history nil)) (with-temp-buffer (insert \"ab\") (defalias 'cl-h (lambda (&rest a) (interactive \"d\") a)) (call-interactively 'cl-h t)) command-history)",
    "(let ((command-history nil)) (defalias 'cl-h (lambda (&rest a) (interactive (list 1 2)) a)) (put 'cl-h 'interactive-args '((1 . (+ 1 1)))) (call-interactively 'cl-h t) command-history)",
    "(let ((command-history nil)) (defalias 'cl-h (lambda (&rest a) (interactive \"U\") a)) (call-interactively 'cl-h) command-history)",
    "(with-temp-buffer (setq buffer-read-only t) (let ((command-history nil)) (defalias 'cl-h (lambda (&rest a) (interactive \"*p\") a)) (list (condition-case e (call-interactively 'cl-h t) (error e)) command-history)))",
    # command-execute (simple.el) drives call-interactively in GNU's loop.
    "(progn (defalias 'cl-ce (lambda (&rest a) (interactive \"p\") (list 'ce a this-command))) (command-execute 'cl-ce))",
    # Non-local exits, recursion, GC.
    "(catch 'cl-tag (call-interactively (lambda () (interactive (throw 'cl-tag 'in-spec)) 1)))",
    "(catch 'cl-tag (call-interactively (lambda () (interactive) (throw 'cl-tag 'in-body))))",
    "(call-interactively (lambda (n) (interactive \"p\") (if (< n 3) (let ((current-prefix-arg (1+ n))) (call-interactively (lambda (m) (interactive \"p\") (list 'inner m)))) n)))",
    f"(let ((x (list 'kept (make-string 2 ?k)))) (list (call-interactively (lambda (a b) (interactive (progn {GC} (list x (make-string 1 ?q)))) {GC} (list (eq a x) b))) x))",
    f"(let ((current-prefix-arg (list 4))) (call-interactively (lambda (a b) (interactive \"P\\np\") {GC} (list a b))))",
    "(list (funcall #'call-interactively (lambda (a) (interactive \"p\") a)) (apply #'call-interactively (list (lambda (a) (interactive \"P\") a))))",
    # Review round 1 (Sol): doc forms, spec snapshot, dotted specs, command-debug-status, replacement-spec environment.
    "(let ((x 0)) (let ((f (function (lambda () (:documentation (setq x 5)) (interactive) 9)))) (list x (condition-case e (call-interactively f) (error e)) x)))",
    "(let ((s (copy-sequence \"^p\"))) (fset 'handle-shift-selection (lambda () (aset s 1 ?P))) (fset 'interactive-form (lambda (f) (list 'interactive s))) (let ((current-prefix-arg '(4))) (call-interactively (lambda (x) x))))",
    "(eval '(call-interactively (function (lambda (a) (interactive \"p\" . 3) a))) t)",
    "(let ((command-debug-status 9)) (call-interactively (lambda () (interactive \"\") command-debug-status)))",
    "(eval '(let ((x 7)) (let ((f (lambda (a) (interactive (list x)) a))) (fset 'interactive-form (lambda (g) '(interactive (list (+ x 1))))) (call-interactively f))) t)",
    # Review round 2 (Sol): dotted (:documentation FORM . X) on a command.
    "(eval '(let ((x 0)) (let ((f (function (lambda () (:documentation (setq x 5) . 3) (interactive) 9)))) (list x (call-interactively f) x))) t)",
]

PROBES = {
    "run-hook-with-args": RUN_HOOK_WITH_ARGS,
    "prefix-numeric-value": PREFIX,
    "funcall-interactively": FUNCALL_INTERACTIVELY,
    "interactive-form": IFORM,
    "call-interactively": CALL_INTERACTIVELY,
}

d = Path(sys.argv[1])
for name, probes in PROBES.items():
    p = d / f"{name}.json"
    spec = json.loads(p.read_text())
    spec["probes"] = probes
    if name == "call-interactively":
        # RECORD-FLAG and KEYS over the symbol catalog (nil, t, 'sym, :kw);
        # the full 19^3 product would be 6859 fresh-process GNU rows of
        # mostly non-command FUNCTIONs. Hand probes cover vectors and records.
        spec["args"] = ["any", "symbol", "symbol"]
    p.write_text(json.dumps(spec) + "\n")
    print(f"{p}: {len(probes)} probes")
