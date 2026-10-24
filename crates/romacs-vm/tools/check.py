#!/usr/bin/env python3
"""Differential semantic checks via GNU's reader and lexical interpreter."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bench import GNU, ROOT, Unscheduled, guarded, load, read_form, validate, COUNTER

CASES = [
    "(+)", "(*)", "(- 9)", "(- 9 3 2)", "(+ 1 2 3 4)", "(* 2 3 4)",
    "(< 1 2 3)", "(< 1 3 2)", "(= 4 4 4)", "(= 4 4 3)",
    "(if nil 12 13)", "(if 0 12 13)", "(if nil 12 (setq x 13) (1+ x))",
    "(progn)", "(setq)", "(while nil 1)", "(car nil)", "(cdr nil)",
    "(let ((x 1)) (let ((x 2) (y x)) (+ x y)))",
    "(let ((x 1)) (let* ((x 2) (y x)) (+ x y)))",
    "(let ((x 1)) (+ x (setq x 4) x))",
    "(let ((x 1)) (setq x 2 x (1+ x)) x)",
    "(let ((x 1)) (let ((f (lambda () x))) (setq x 42) (funcall f)))",
    "(let ((x 1)) (let ((f (lambda () (setq x (1+ x))))) (funcall f) (funcall f) x))",
    "(let* ((x 1) (f (lambda () (setq x (1+ x))))) (funcall f) x)",
    "(let ((f (let ((x 1)) (lambda () (setq x (1+ x)))))) (funcall f) (funcall f))",
    "(let ((x 1)) (let ((make (lambda (y) (lambda () (+ x y))))) (funcall (funcall make 4))))",
    "(let ((x 1)) (let ((f (lambda (x) (setq x (1+ x)) x))) (+ (funcall f 4) x)))",
    "(let ((x 1)) (defun local-f () (setq x (1+ x))) (local-f) (local-f) x)",
    "(progn (defun ident (x) x) (funcall 'ident 19))",
    "(progn (defun ident (x) x) (funcall #'ident 19))",
    "(progn (defun target () 1) (defun caller () (target)) (caller) (defun target () 2) (caller))",
    "(progn (defun + (a b) (- a b)) (+ 9 4))",
    "(let ((f (lambda (x) (lambda () (setq x (1+ x)))))) (let ((g (funcall f 9))) (funcall g) (funcall g)))",
    "(let ((l (cons 1 (cons 2 (cons 3 nil))))) (+ (car l) (car (cdr l)) (length l)))",
    "(let ((l (cons 1 (cons 2 (cons 3 nil))))) (setq l (nreverse l)) (+ (* 100 (car l)) (* 10 (car (cdr l))) (car (cdr (cdr l)))))",
    "(let ((i 0) (fs nil)) (while (< i 3) (let ((x i)) (setq fs (cons (lambda () x) fs))) (setq i (1+ i))) (+ (funcall (car fs)) (* 10 (funcall (car (cdr fs))))))",
    "(progn (defun fib (n) (if (< n 2) n (+ (fib (1- n)) (fib (- n 2))))) (fib 12))",
    "(progn (defun f () 1) 'f)",
]
ERRORS = ["(+ nil 2)", "(car 1)", "(length (cons 1 2))", "(funcall 1)",
          "(funcall (lambda (x) x))", "(setq t 2)", "(let ((nil 1)) nil)",
          "(1+ 1152921504606846975)", "(+ 1152921504606846975 1)",
          "(funcall (lambda (x &rest args) args))", "unbound-name", "(unknown-function 1)"]
SPECIALS = [
    "(progn (defun get-s () s) (+ (let ((s 9)) (get-s)) s))",
    "(progn (defun set-s (s) (setq s (1+ s)) s) (+ (set-s 9) s))",
    "(let ((f (let ((s 9)) (lambda () s)))) (funcall f))",
    "(let ((s 9)) (+ s (let* ((s 11) (x s)) x) s))",
    "(let ((s 9)) (let ((s 11) (x s)) x))",
]


class Semantics(unittest.TestCase):
    def test_forms(self):
        for source in CASES + SPECIALS:
            with self.subTest(source=source):
                special = source in SPECIALS
                data = read_form(source)
                command = [BINARY] + (["--special", "s=3"] if special else [])
                got = subprocess.check_output(command, input=data).decode().strip()
                prelude = "(defvar s 3)" if special else "nil"
                expected = subprocess.check_output([GNU, "-Q", "--batch", "--eval", prelude, "--eval",
                    f"(princ (eval (read {json.dumps(source)}) t))"], text=True).strip()
                self.assertEqual(got, expected)

    def test_errors(self):
        for source in ERRORS:
            with self.subTest(source=source):
                result = subprocess.run([BINARY], input=read_form(source), capture_output=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, b"")
                self.assertIn(b"vm2:", result.stderr)

    def test_counter_guard(self):
        for values in ((0, 1000, 1000), (1, 0, 0), (1, 1000, 998), (1, 1000, 1001)):
            with self.subTest(values=values), self.assertRaises(Unscheduled):
                validate(*values)
        with self.assertRaises(Unscheduled):
            validate(1, 1000, 999)  # Even 99.9% scheduling is forbidden for S1.
        validate(1, 1000, 1000)

    def test_real_contention_is_refused(self):
        with self.assertRaises(Unscheduled) as rejection:
            guarded([BINARY], b'42', load(COUNTER, "pressure_counter").attributes, pressure=64)
        print(f"\nPMU pressure refused: {rejection.exception}")

    def test_real_counter(self):
        result = guarded([BINARY], b'42', load(COUNTER, "counter").attributes)
        self.assertEqual(result["stdout"], "42")
        self.assertGreater(result["count"], 0)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=str(ROOT / "target/release/romacs-vm"))
    args, rest = parser.parse_known_args()
    BINARY = args.binary
    unittest.main(argv=[sys.argv[0]] + rest)
