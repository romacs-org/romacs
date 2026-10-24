#!/usr/bin/env python3
"""Write the primh specs for the window primitives (lane vm2/window-prims).

The specs only DECLARE argument domains and probes; every answer comes from
the pinned GNU through `record.py primh` (or `primh.py record` with the pin
binary). The set is MEASURED, not chosen: `window-specs/MEASURED.md` records
how GNU's own window.el, driven through the journey, reached each one.

Declaration fields (name, arity, effect, gnu_def, rust) are read from the
production manifest, so a moved declaration re-generates, never drifts.

Geometry differs between the oracle and VM2 by construction: GNU -Q --batch
runs an 80x25 terminal frame whose canonical character is one pixel, VM2 a
640x480 frame with 20x10-pixel characters. This is a documented HYBRID device
model (VM2-WIN5), not graphical/terminal parity. The joint frame-live-p probe
records that mismatch explicitly. VM2-WIN4 likewise pins the scrolled lower
live window path of delete-other-windows-internal; its start is not adjusted.
Other geometry probes state sizes as
relations (`(= (window-new-total b) (/ (window-pixel-height p) (frame-char-height)))`)
and never print a window or frame: windows are compared with `eq` and named
by letter, and signal data that holds a window is reported as `(eq DATUM W)`.

`round' and `ceiling' (floatfns.c) are here because the same measurement
reached them: split-window rounds sizes to character multiples.

Usage: python3 window_specs.py MANIFEST   (rewrites window-specs/*.json)
"""
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "window-specs"

# One window split below: A (selected, top) and B (new, bottom) under P.
def vsplit(body):
    return ("(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2))"
            " (b (progn (set-window-new-pixel a (- h half)) (split-window-internal a half nil 0.5)))"
            " (p (window-parent b))) " + body + ")")

# One window split right: A (left) and B (right) under P.
def hsplit(body):
    return ("(let* ((a (selected-window)) (h (window-pixel-width a)) (half (/ h 2))"
            " (b (progn (set-window-new-pixel a (- h half)) (split-window-internal a half t 0.5)))"
            " (p (window-parent b))) " + body + ")")

# After a vertical split, delete B again: B and P are dead (P is replaced by A).
def dead(body):
    return vsplit("(progn (set-window-new-pixel p h) (set-window-new-pixel a h)"
                  " (delete-window-internal b) " + body + ")")

# Name a window (or a list of them) by its role.
NAMES = ("(lambda (w) (cond ((eq w a) 'a) ((eq w b) 'b) ((eq w p) 'p)"
         " ((eq w (minibuffer-window)) 'm) ((windowp w) 'other) (t w)))")

# A condition whose data holds OBJECT: (CONDITION PREDICATE (eq DATUM OBJECT)).
def sig_eq(form, obj):
    return (f"(condition-case e {form} (error (list (car e) (car (cdr e))"
            f" (eq (car (cdr (cdr e))) {obj}))))")

# VM2 has no `mapcar' yet; every `(mapcar F L)' in a probe is rewritten to a
# dolist so the probes measure the window primitives, not the mapper.
def sexp_end(text, i):
    depth, j, string = 0, i, False
    while True:
        ch = text[j]
        if string:
            if ch == "\\":
                j += 1
            elif ch == '"':
                string = False
        elif ch == '"':
            string = True
        elif ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return j + 1
        elif depth == 0 and ch in " )":
            return j
        j += 1


def demapcar(text):
    while "(mapcar " in text:
        i = text.index("(mapcar ")
        f0 = i + len("(mapcar ")
        f1 = sexp_end(text, f0)
        l0 = f1 + 1
        l1 = sexp_end(text, l0)
        assert text[l1] == ")", text[i:l1 + 1]
        fn, lst = demapcar(text[f0:f1]), demapcar(text[l0:l1])
        text = text[:i] + f"(let (r) (dolist (w {lst}) (push (funcall {fn} w) r)) (nreverse r))" + text[l1 + 1:]
    return text

BAD = ["1", "'sym", "(copy-sequence \"s\")"]

# `run-hook-with-args' (eval.c) is not yet a VM2 primitive on this branch's
# base. A probe that needs it installs this spy first: a no-op where GNU (or
# a later VM2) defines it, never a production definition.
HOOK_SPY = ("(unless (fboundp 'run-hook-with-args)"
            " (defalias 'run-hook-with-args (lambda (hook &rest args) (apply (symbol-value hook) args))))")

SPECS = {
    "window-valid-p": dict(args=["none"], extra={"0": ["nil", "t"] + BAD}, probes=[
        "(window-valid-p (selected-window))",
        "(window-valid-p (minibuffer-window))",
        "(window-valid-p (frame-root-window))",
        "(window-valid-p (selected-frame))",
        vsplit("(list (window-valid-p p) (window-valid-p a) (window-valid-p b))"),
        dead("(list (window-valid-p b) (window-valid-p p) (window-valid-p a))"),
        "(funcall #'window-valid-p (selected-window))",
    ]),
    "window-frame": dict(args=["none"], extra={"0": BAD + ["t"]}, probes=[
        "(eq (window-frame) (selected-frame))",
        "(eq (window-frame (selected-window)) (selected-frame))",
        "(eq (window-frame (minibuffer-window)) (selected-frame))",
        vsplit("(list (eq (window-frame p) (selected-frame)) (eq (window-frame b) (window-frame a)))"),
        dead(sig_eq("(window-frame b)", "b")),
        dead(sig_eq("(window-frame p)", "p")),
        sig_eq("(window-frame (selected-frame))", "(selected-frame)"),
        "(eq (funcall #'window-frame) (selected-frame))",
    ]),
    "window-minibuffer-p": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-minibuffer-p)",
        "(window-minibuffer-p (minibuffer-window))",
        "(window-minibuffer-p (selected-window))",
        vsplit("(list (window-minibuffer-p p) (window-minibuffer-p b))"),
        dead(sig_eq("(window-minibuffer-p b)", "b")),
        sig_eq("(window-minibuffer-p (selected-frame))", "(selected-frame)"),
    ]),
    "set-frame-selected-window": dict(args=["none", "none", "none"], extra={"0": ["1"], "1": ["nil", "1"], "2": ["nil"]}, probes=[
        "(eq (set-frame-selected-window nil (selected-window)) (selected-window))",
        vsplit("(list (eq (set-frame-selected-window nil b) b) (eq (selected-window) b) (eq (frame-selected-window) b) (eq (current-buffer) (window-buffer b)))"),
        vsplit("(list (window-use-time b) (progn (set-frame-selected-window nil b) (- (window-use-time b) (window-use-time a))))"),
        vsplit("(list (window-use-time b) (progn (set-frame-selected-window nil b t) (window-use-time b)) (eq (selected-window) b))"),
        vsplit("(eq (set-frame-selected-window (selected-frame) b) b)"),
        vsplit("(eq (set-frame-selected-window (window-frame b) b 'norecord) b)"),
        vsplit(sig_eq("(set-frame-selected-window nil p)", "p")),
        dead(sig_eq("(set-frame-selected-window nil b)", "b")),
        "(condition-case e (set-frame-selected-window 1 1) (error e))",
        "(condition-case e (set-frame-selected-window 'sym (selected-window)) (error e))",
        sig_eq("(set-frame-selected-window nil (selected-frame))", "(selected-frame)"),
        "(eq (set-frame-selected-window nil (minibuffer-window)) (minibuffer-window))",
    ]),
    "window-top-child": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-top-child)",
        "(window-top-child (minibuffer-window))",
        vsplit(f"(list (funcall {NAMES} (window-top-child p)) (window-top-child a) (window-top-child b))"),
        hsplit("(list (window-top-child p) (eq (window-left-child p) a))"),
        vsplit(f"(let* ((c (progn (set-window-new-pixel b (- (window-pixel-width b) (/ (window-pixel-width b) 2))) (split-window-internal b (/ (window-pixel-width b) 2) t 0.5))) (q (window-parent c))) (list (funcall {NAMES} (window-top-child p)) (eq (window-next-sibling a) q) (window-top-child q) (eq (window-left-child q) b)))"),
        dead(sig_eq("(window-top-child p)", "p")),
        sig_eq("(window-top-child (selected-frame))", "(selected-frame)"),
    ]),
    "window-left-child": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-left-child)",
        "(window-left-child (minibuffer-window))",
        hsplit(f"(list (funcall {NAMES} (window-left-child p)) (window-left-child a) (window-left-child b))"),
        vsplit("(window-left-child p)"),
        dead(sig_eq("(window-left-child b)", "b")),
        sig_eq("(window-left-child (selected-frame))", "(selected-frame)"),
    ]),
    "window-use-time": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-use-time)",
        "(window-use-time (minibuffer-window))",
        vsplit("(list (window-use-time a) (window-use-time b))"),
        vsplit("(progn (select-window b) (list (window-use-time a) (window-use-time b)))"),
        vsplit("(progn (select-window b t) (list (window-use-time a) (window-use-time b)))"),
        vsplit("(progn (select-window a) (select-window a) (list (window-use-time a) (window-use-time b)))"),
        vsplit("(progn (select-window b) (select-window a) (select-window b) (list (window-use-time a) (window-use-time b)))"),
        vsplit(sig_eq("(window-use-time p)", "p")),
        dead(sig_eq("(window-use-time b)", "b")),
    ]),
    "window-new-total": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-new-total)",
        "(window-new-total (minibuffer-window))",
        "(window-new-total (frame-root-window))",
        vsplit("(list (window-new-total a) (= (window-new-total b) (/ (window-pixel-height p) (frame-char-height))) (= (window-new-total p) (/ h (frame-char-height))))"),
        hsplit("(list (window-new-total a) (= (window-new-total b) (/ (window-pixel-width p) (frame-char-width))) (= (window-new-total p) (/ h (frame-char-width))))"),
        vsplit("(progn (set-window-new-total a 3) (let* ((c (progn (set-window-new-pixel b (- (window-pixel-height b) (/ (window-pixel-height b) 2))) (split-window-internal b (/ (window-pixel-height b) 2) nil 0.25)))) (list (window-new-total a) (= (window-new-total c) (- (/ h (frame-char-height)) 3 (window-new-total b))))))"),
        dead("(list (window-new-total a) (window-valid-p b))"),
        dead(sig_eq("(window-new-total b)", "b")),
    ]),
    "set-window-new-total": dict(args=["none", "none", "none"], extra={"0": ["1"], "1": ["1.5", "'x"], "2": ["nil", "t"]}, probes=[
        "(set-window-new-total nil 5)",
        "(progn (set-window-new-total nil 5) (list (set-window-new-total nil 3 t) (window-new-total)))",
        "(progn (set-window-new-total nil 5) (list (set-window-new-total nil -7 'add) (window-new-total)))",
        "(progn (set-window-new-total nil 5) (list (set-window-new-total nil 2 nil) (window-new-total)))",
        "(list (set-window-new-total (minibuffer-window) 9) (window-new-total (minibuffer-window)) (window-new-total))",
        "(condition-case e (set-window-new-total nil 1.5) (error e))",
        "(condition-case e (set-window-new-total nil 'x t) (error e))",
        "(condition-case e (set-window-new-total nil (1+ most-positive-fixnum)) (error (car e)))",
        "(condition-case e (set-window-new-total 1 'x) (error e))",
        vsplit("(list (set-window-new-total p 11) (window-new-total p) (window-new-total a))"),
        dead(sig_eq("(set-window-new-total b 1)", "b")),
        "(let ((n (set-window-new-total nil 4))) (eq n (window-new-total)))",
    ]),
    "window-normal-size": dict(args=["none", "none"], extra={"0": BAD, "1": ["nil"]}, probes=[
        "(window-normal-size)",
        "(window-normal-size nil t)",
        "(window-normal-size (minibuffer-window))",
        "(list (window-normal-size (minibuffer-window) t) (window-normal-size nil nil))",
        vsplit("(list (window-normal-size a) (window-normal-size b) (window-normal-size p) (window-normal-size a t) (window-normal-size b t) (window-normal-size p t))"),
        hsplit("(list (window-normal-size a t) (window-normal-size b t) (window-normal-size p t) (window-normal-size a) (window-normal-size b))"),
        "(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) (b (progn (set-window-new-pixel a (- h half)) (set-window-new-normal a 0.75) (split-window-internal a half nil 0.25)))) (list (window-normal-size a) (window-normal-size b) (window-normal-size (window-parent a))))",
        "(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) (b (progn (set-window-new-pixel a (- h half)) (set-window-new-normal a 'sym) (split-window-internal a half nil 'other)))) (list (window-normal-size a) (window-normal-size b) (window-new-normal b)))",
        "(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) (b (progn (set-window-new-pixel a (- h half)) (set-window-new-normal a 3) (split-window-internal a half 'above 0.5)))) (list (window-normal-size a) (window-normal-size b) (eq (window-top-child (window-parent a)) b)))",
        vsplit("(progn (set-window-new-normal a 0.25) (set-window-new-normal b 0.75) (set-window-new-pixel p h) (set-window-new-pixel a half) (set-window-new-pixel b (- h half)) (list (window-resize-apply) (window-normal-size a) (window-normal-size b) (window-normal-size a t)))"),
        vsplit("(progn (set-window-new-normal a 0.25) (set-window-new-pixel p h) (set-window-new-pixel a h) (delete-window-internal b) (list (window-normal-size a) (window-parent a)))"),
        dead(sig_eq("(window-normal-size b)", "b")),
        "(eq (window-normal-size) (window-normal-size))",
    ]),
    "window-new-normal": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-new-normal)",
        "(window-new-normal (minibuffer-window))",
        vsplit("(list (window-new-normal a) (window-new-normal b) (window-new-normal p))"),
        "(let* ((a (selected-window)) (h (window-pixel-height a)) (half (/ h 2)) (b (progn (set-window-new-pixel a (- h half)) (set-window-new-normal a 0.75) (split-window-internal a half nil 0.25)))) (list (window-new-normal a) (window-new-normal b) (window-new-normal (window-parent a))))",
        dead(sig_eq("(window-new-normal b)", "b")),
    ]),
    "set-window-new-normal": dict(args=["none", "none"], extra={"0": ["1"], "1": ["nil", "1.5"]}, probes=[
        "(set-window-new-normal nil 0.5)",
        "(list (set-window-new-normal nil 'anything) (window-new-normal))",
        "(list (set-window-new-normal nil) (window-new-normal))",
        "(let ((x (list 1 2))) (eq (progn (set-window-new-normal nil x) (window-new-normal)) x))",
        "(list (set-window-new-normal (minibuffer-window) 2) (window-new-normal (minibuffer-window)) (window-new-normal))",
        vsplit("(list (set-window-new-normal p 0.1) (window-new-normal p))"),
        dead(sig_eq("(set-window-new-normal b 0.5)", "b")),
        "(condition-case e (set-window-new-normal 'x 0.5) (error e))",
    ]),
    "window-resize-apply-total": dict(args=["none", "none"], extra={"0": ["1", "'sym"], "1": ["nil"]}, probes=[
        "(window-resize-apply-total)",
        "(window-resize-apply-total nil t)",
        "(window-resize-apply-total (selected-frame))",
        sig_eq("(window-resize-apply-total (selected-window))", "(selected-window)"),
        vsplit("(list (window-resize-apply-total) (window-live-p a) (window-live-p b) (eq (window-parent a) p))"),
        "(progn (set-window-new-total (frame-root-window) 7) (window-resize-apply-total) (window-total-height))",
        "(progn (set-window-new-total (frame-root-window) (window-total-height)) (set-window-new-total (minibuffer-window) 1) (list (window-resize-apply-total) (window-total-height (minibuffer-window))))",
    ]),
    "window-mode-line-height": dict(args=["none"], extra={"0": BAD}, probes=[
        "(= (window-mode-line-height) (frame-char-height))",
        "(window-mode-line-height (minibuffer-window))",
        "(let ((mode-line-format nil)) (window-mode-line-height))",
        "(progn (set-window-parameter nil 'mode-line-format 'none) (window-mode-line-height))",
        "(let ((mode-line-format nil)) (set-window-parameter nil 'mode-line-format \"x\") (= (window-mode-line-height) (frame-char-height)))",
        vsplit("(list (= (window-mode-line-height a) (frame-char-height)) (= (window-mode-line-height b) (frame-char-height)))"),
        vsplit(sig_eq("(window-mode-line-height p)", "p")),
        dead(sig_eq("(window-mode-line-height b)", "b")),
    ]),
    "window-header-line-height": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-header-line-height)",
        "(window-header-line-height (minibuffer-window))",
        vsplit(sig_eq("(window-header-line-height p)", "p")),
        dead(sig_eq("(window-header-line-height b)", "b")),
        "(let ((header-line-format \"x\")) (window-header-line-height))",
    ]),
    "window-tab-line-height": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-tab-line-height)",
        "(window-tab-line-height (minibuffer-window))",
        vsplit(sig_eq("(window-tab-line-height p)", "p")),
        dead(sig_eq("(window-tab-line-height b)", "b")),
    ]),
    "window-right-divider-width": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-right-divider-width)",
        "(window-right-divider-width (minibuffer-window))",
        hsplit("(list (window-right-divider-width a) (window-right-divider-width b))"),
        vsplit(sig_eq("(window-right-divider-width p)", "p")),
        dead(sig_eq("(window-right-divider-width b)", "b")),
    ]),
    "window-bottom-divider-width": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-bottom-divider-width)",
        "(window-bottom-divider-width (minibuffer-window))",
        vsplit("(list (window-bottom-divider-width a) (window-bottom-divider-width b))"),
        vsplit(sig_eq("(window-bottom-divider-width p)", "p")),
        dead(sig_eq("(window-bottom-divider-width b)", "b")),
    ]),
    "window-scroll-bar-width": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-scroll-bar-width)",
        "(window-scroll-bar-width (minibuffer-window))",
        vsplit(sig_eq("(window-scroll-bar-width p)", "p")),
        dead(sig_eq("(window-scroll-bar-width b)", "b")),
    ]),
    "window-scroll-bar-height": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-scroll-bar-height)",
        "(window-scroll-bar-height (minibuffer-window))",
        vsplit(sig_eq("(window-scroll-bar-height p)", "p")),
        dead(sig_eq("(window-scroll-bar-height b)", "b")),
    ]),
    "window-dedicated-p": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-dedicated-p)",
        "(window-dedicated-p (minibuffer-window))",
        vsplit("(list (window-dedicated-p a) (window-dedicated-p b))"),
        vsplit(sig_eq("(window-dedicated-p p)", "p")),
        dead(sig_eq("(window-dedicated-p b)", "b")),
    ]),
    "next-window": dict(args=["none", "none", "none"], extra={"0": BAD, "1": ["nil"], "2": ["nil"]}, probes=[
        "(eq (next-window) (selected-window))",
        "(eq (next-window nil t) (minibuffer-window))",
        "(eq (next-window (minibuffer-window)) (selected-window))",
        "(eq (next-window (minibuffer-window) t) (selected-window))",
        vsplit(f"(mapcar {NAMES} (list (next-window a) (next-window b) (next-window) (next-window b t) (next-window b 'lambda) (next-window (minibuffer-window)) (next-window (minibuffer-window) t)))"),
        vsplit(f"(mapcar {NAMES} (list (next-window a nil t) (next-window b nil 'visible) (next-window b nil 0) (next-window b nil (selected-frame)) (next-window b nil 'other) (next-window b t t)))"),
        vsplit(f"(mapcar {NAMES} (list (next-window a nil a) (next-window b nil b) (next-window b nil (minibuffer-window)) (next-window b t (minibuffer-window)) (next-window a 'lambda (minibuffer-window))))"),
        vsplit(f"(let* ((c (progn (set-window-new-pixel b (- (window-pixel-width b) (/ (window-pixel-width b) 2))) (split-window-internal b (/ (window-pixel-width b) 2) t 0.5)))) (mapcar (lambda (w) (cond ((eq w a) 'a) ((eq w b) 'b) ((eq w c) 'c) ((eq w (minibuffer-window)) 'm) (t w))) (list (next-window a) (next-window b) (next-window c) (next-window c t) (next-window (minibuffer-window) t))))"),
        vsplit(f"(let* ((c (progn (set-window-new-pixel a (- (window-pixel-height a) (/ (window-pixel-height a) 2))) (split-window-internal a (/ (window-pixel-height a) 2) 'above 0.5)))) (mapcar (lambda (w) (cond ((eq w a) 'a) ((eq w b) 'b) ((eq w c) 'c) ((eq w (minibuffer-window)) 'm) (t w))) (list (next-window c) (next-window a) (next-window b))))"),
        vsplit(sig_eq("(next-window p)", "p")),
        dead(sig_eq("(next-window b)", "b")),
        dead(f"(mapcar {NAMES} (list (next-window a) (next-window a t)))"),
        sig_eq("(next-window (selected-frame))", "(selected-frame)"),
        "(eq (funcall #'next-window) (selected-window))",
    ]),
    "window-list-1": dict(args=["none", "none", "none"], extra={"0": BAD, "1": ["nil"], "2": ["nil"]}, probes=[
        "(equal (window-list-1) (list (selected-window)))",
        "(equal (window-list-1 nil t) (list (selected-window) (minibuffer-window)))",
        "(equal (window-list-1 (minibuffer-window) t) (list (minibuffer-window) (selected-window)))",
        "(equal (window-list-1 (minibuffer-window)) (list (selected-window)))",
        vsplit(f"(list (mapcar {NAMES} (window-list-1)) (mapcar {NAMES} (window-list-1 b)) (mapcar {NAMES} (window-list-1 b t)) (mapcar {NAMES} (window-list-1 a 'lambda)) (mapcar {NAMES} (window-list-1 (minibuffer-window) t)))"),
        vsplit(f"(list (mapcar {NAMES} (window-list-1 b nil t)) (mapcar {NAMES} (window-list-1 b t 'visible)) (mapcar {NAMES} (window-list-1 b nil a)) (mapcar {NAMES} (window-list-1 b nil (minibuffer-window))) (mapcar {NAMES} (window-list-1 b t (minibuffer-window))))"),
        hsplit(f"(let* ((c (progn (set-window-new-pixel a (- (window-pixel-height a) (/ (window-pixel-height a) 2))) (split-window-internal a (/ (window-pixel-height a) 2) nil 0.5)))) (mapcar (lambda (w) (cond ((eq w a) 'a) ((eq w b) 'b) ((eq w c) 'c) ((eq w (minibuffer-window)) 'm) (t w))) (window-list-1 b t)))"),
        vsplit(sig_eq("(window-list-1 p)", "p")),
        dead(sig_eq("(window-list-1 b)", "b")),
        "(let ((l (window-list-1))) (eq l (window-list-1)))",
    ]),
    "delete-other-windows-internal": dict(args=["none", "none"], extra={"0": BAD, "1": ["nil"]}, probes=[
        "(delete-other-windows-internal)",
        "(condition-case e (delete-other-windows-internal (minibuffer-window)) (error e))",
        vsplit("(list (delete-other-windows-internal a) (window-live-p a) (window-live-p b) (window-valid-p p) (eq (frame-root-window) a) (eq (selected-window) a) (window-parent a) (= (window-pixel-height a) h))"),
        vsplit("(progn (select-window a) (list (delete-other-windows-internal b) (window-live-p a) (window-live-p b) (eq (frame-root-window) b) (eq (selected-window) b) (eq (current-buffer) (window-buffer b)) (= (window-pixel-height b) h)))"),
        vsplit("(let ((before (window-use-time a))) (delete-other-windows-internal b) (list (- (window-use-time b) before) (window-live-p b)))"),
        hsplit("(progn (delete-other-windows-internal b) (list (eq (frame-root-window) b) (= (window-pixel-width b) h) (window-live-p a)))"),
        vsplit("(let* ((c (progn (set-window-new-pixel b (- (window-pixel-width b) (/ (window-pixel-width b) 2))) (split-window-internal b (/ (window-pixel-width b) 2) t 0.5))) (q (window-parent c))) (list (delete-other-windows-internal q) (eq (frame-root-window) q) (window-live-p a) (window-live-p b) (window-live-p c) (= (+ (window-pixel-width b) (window-pixel-width c)) (window-pixel-width q)) (= (window-pixel-height b) (window-pixel-height q)) (= (window-pixel-height q) h) (eq (selected-window) b)))"),
        vsplit("(let* ((c (progn (set-window-new-pixel b (- (window-pixel-width b) (/ (window-pixel-width b) 2))) (split-window-internal b (/ (window-pixel-width b) 2) t 0.5))) (q (window-parent c))) (list (delete-other-windows-internal c q) (eq (window-next-sibling a) c) (eq (window-parent c) p) (window-live-p b) (window-valid-p q) (eq (frame-root-window) p)))"),
        vsplit("(let* ((c (progn (set-window-new-pixel b (- (window-pixel-width b) (/ (window-pixel-width b) 2))) (split-window-internal b (/ (window-pixel-width b) 2) t 0.5))) (q (window-parent c))) (condition-case e (delete-other-windows-internal a q) (error e)))"),
        vsplit("(delete-other-windows-internal p)"),
        vsplit("(delete-other-windows-internal a a)"),
        dead(sig_eq("(delete-other-windows-internal b)", "b")),
        vsplit(sig_eq("(delete-other-windows-internal a 1)", "1")),
        vsplit("(progn (set-window-point b 1) (delete-other-windows-internal a) (list (window-start a) (window-live-p a)))"),
        # VM2-WIN4: pin as VALUE; GNU uses vmotion to preserve the old screen position.
        vsplit('(progn (let ((i 0)) (while (< i 100) (insert "line\\n") (setq i (1+ i)))) (set-window-start b 201) (let ((old (window-start b))) (delete-other-windows-internal b) (list old (window-start b) (eq (frame-root-window) b))))'),
    ]),
    "run-window-scroll-functions": dict(args=["none"], extra={"0": BAD}, probes=[
        "(run-window-scroll-functions)",
        "(run-window-scroll-functions (minibuffer-window))",
        "(let ((window-scroll-functions nil)) (run-window-scroll-functions (selected-window)))",
        "(progn " + HOOK_SPY + " (let* ((other (get-buffer-create \"rwsf\")) (seen nil)) (set-buffer other) (let ((window-scroll-functions (lambda (w s) (setq seen (list (eq w (selected-window)) s (eq (current-buffer) (window-buffer w)))) 'ignored))) (list (run-window-scroll-functions) seen (eq (current-buffer) other)))))",
        "(progn " + HOOK_SPY + " (let* ((seen nil) (w0 (selected-window))) (let ((window-scroll-functions (lambda (w s) (setq seen (list (eq w w0) s))))) (list (run-window-scroll-functions w0) seen))))",
        "(progn " + HOOK_SPY + " (let* ((other (get-buffer-create \"rwsf\")) (seen nil)) (set-buffer other) (condition-case e (let ((window-scroll-functions (lambda (w s) (setq seen s) (car 1)))) (run-window-scroll-functions)) (error (list e seen (eq (current-buffer) other))))))",
        vsplit(sig_eq("(run-window-scroll-functions p)", "p")),
        dead(sig_eq("(run-window-scroll-functions b)", "b")),
    ]),
    "window-fringes": dict(args=["none"], extra={"0": BAD}, probes=[
        "(window-fringes)",
        "(window-fringes (minibuffer-window))",
        vsplit("(list (window-fringes a) (window-fringes b))"),
        vsplit(sig_eq("(window-fringes p)", "p")),
        dead(sig_eq("(window-fringes b)", "b")),
        "(eq (window-fringes) (window-fringes))",
    ]),
    "frame-live-p": dict(args=["none"], extra={"0": ["nil", "t"] + BAD}, probes=[
        "(frame-live-p (selected-frame))",
        "(frame-live-p (window-frame (minibuffer-window)))",
        "(frame-live-p (selected-window))",
        "(funcall #'frame-live-p (selected-frame))",
        # VM2-WIN5: terminal type and graphical metrics are a VALUE divergence.
        "(list (frame-live-p (selected-frame)) (frame-char-height) (frame-char-width))",
    ]),
    "frame-char-height": dict(args=["none"], extra={"0": BAD}, probes=[
        "(integerp (frame-char-height))",
        "(= (frame-char-height) (frame-char-height (selected-frame)))",
        "(= (* (frame-char-height) (window-total-height (minibuffer-window))) (window-pixel-height (minibuffer-window)))",
        "(> (frame-char-height) 0)",
        sig_eq("(frame-char-height (selected-window))", "(selected-window)"),
    ]),
    "frame-char-width": dict(args=["none"], extra={"0": BAD}, probes=[
        "(integerp (frame-char-width))",
        "(= (frame-char-width) (frame-char-width (selected-frame)))",
        "(= (* (frame-char-width) (window-total-width (selected-window))) (window-pixel-width (selected-window)))",
        sig_eq("(frame-char-width (selected-window))", "(selected-window)"),
    ]),
    "frame-right-divider-width": dict(args=["none"], extra={"0": BAD}, probes=[
        "(frame-right-divider-width)",
        "(frame-right-divider-width (selected-frame))",
        sig_eq("(frame-right-divider-width (selected-window))", "(selected-window)"),
    ]),
    "frame-bottom-divider-width": dict(args=["none"], extra={"0": BAD}, probes=[
        "(frame-bottom-divider-width)",
        "(frame-bottom-divider-width (selected-frame))",
        sig_eq("(frame-bottom-divider-width (selected-window))", "(selected-window)"),
    ]),
    "active-minibuffer-window": dict(args=[], probes=[
        "(active-minibuffer-window)",
        "(funcall #'active-minibuffer-window)",
        "(progn (defvar romacs-amw-seen 'unset) (defun romacs-amw-spy () (interactive) (setq romacs-amw-seen (eq (active-minibuffer-window) (minibuffer-window)))) (defvar romacs-amw-map (let ((m (make-sparse-keymap))) (set-keymap-parent m minibuffer-local-map) (define-key m [f5] 'romacs-amw-spy) m)) (let ((unread-command-events (append [f5 ?\\r] nil)) (executing-kbd-macro t)) (list (read-from-minibuffer \"P: \" nil romacs-amw-map) romacs-amw-seen (active-minibuffer-window))))",
    ]),
}

NUM0 = ["2.5", "-2.5", "0.5", "-0.5", "-0.0", "1.0e+INF", "-1.0e+INF", "0.0e+NaN", "1e300", "5e-324",
        "most-negative-fixnum", "(- most-positive-fixnum)", "7", "-7", "230"]
NUM1 = ["0.0", "-0.0", "2", "-2", "3", "20", "0.5", "0.1", "1.0e+INF", "0.0e+NaN", "1e-300", "most-negative-fixnum"]
ROUNDING = [
    "(list (FN 0.5) (FN 1.5) (FN 2.5) (FN -0.5) (FN -1.5) (FN -2.5) (FN 3.49) (FN -3.51))",
    "(list (FN 7 2) (FN -7 2) (FN 7 -2) (FN -7 -2) (FN 5 2) (FN -5 2) (FN 6 4) (FN 10 4) (FN -10 4))",
    "(list (FN 1.5 0.5) (FN 0.1 0.01) (FN 0.3 0.1) (FN 1 3.0) (FN 2 3.0) (FN -2 3.0) (FN 1 0.1))",
    "(list (FN 5e-324 1e300) (FN -5e-324 1e300) (FN 1e-300 1e300) (FN 1 1.0e+INF) (FN -1 1.0e+INF) (FN 0.0 1.0e+INF))",
    "(list (FN 1e300 1e299) (FN 1e15 0.1) (FN 4503599627370497.0 2) (FN 9007199254740993 2.0))",
    "(list (FN 230 20) (* 20 (FN 230 20)) (FN 460 2) (FN 12 1) (FN 25 2) (FN 23 2))",
    "(condition-case e (FN most-negative-fixnum -1) (error e))",
    "(condition-case e (FN most-positive-fixnum 0.5) (error e))",
    "(condition-case e (FN 1e300) (error e))",
    "(condition-case e (FN 1.0e+INF) (error e))",
    "(condition-case e (FN 0.0e+NaN) (error e))",
    "(condition-case e (FN 1.0e+INF 1.0e+INF) (error e))",
    "(condition-case e (FN 1 0.0e+NaN) (error e))",
    "(condition-case e (FN 1 0) (error e))",
    "(condition-case e (FN 1.5 -0.0) (error e))",
    "(condition-case e (FN (make-marker)) (error (car e)))",
    "(condition-case e (FN 1 'x) (error e))",
    "(condition-case e (FN 'x 0) (error e))",
    "(let ((x 1.5)) (list (FN x) x))",
    "(eq (FN most-positive-fixnum) most-positive-fixnum)",
    "(funcall #'FN 9 4)",
    "(apply #'FN '(9.5))",
]
SPECS["round"] = dict(args=["any", "any"], extra={"0": NUM0, "1": NUM1},
                      probes=[p.replace("FN", "round") for p in ROUNDING])
SPECS["ceiling"] = dict(args=["any", "any"], extra={"0": NUM0, "1": NUM1},
                        probes=[p.replace("FN", "ceiling") for p in ROUNDING])


def main(manifest):
    man = {m["name"]: m for m in json.loads(Path(manifest).read_text())}
    OUT.mkdir(exist_ok=True)
    for name, body in SPECS.items():
        m = man[name]
        body = dict(body, probes=[demapcar(p) for p in body["probes"]])
        spec = dict(name=name, arity=[m["min"], m["max"]], effect=m["effect"].lower(),
                    gnu_files=[m["gnu"].split(":")[0]], gnu_def=m["gnu"], rust=m["rust"], **body)
        (OUT / f"{name}.json").write_text(json.dumps(spec, indent=1, ensure_ascii=False) + "\n")
    print(f"wrote {len(SPECS)} specs to {OUT}")


if __name__ == "__main__":
    main(sys.argv[1])
