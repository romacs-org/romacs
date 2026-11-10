#!/usr/bin/env python3
"""Record reverse answers using the revision-checked project GNU oracle."""
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools/lib"))
import gnu_oracle

CASES = [
    ("callable", "(list (fboundp 'reverse) (functionp 'reverse) (funcall 'reverse '(1 2 3)))"),
    ("lasso", "(let ((x (list 1 2 3 4))) (setcdr (cdr (cdr (cdr x))) (cdr x)) (condition-case e (reverse x) (circular-list (list (car e) (car (car (cdr e))) (car x) (car (cdr x))))))"),
    ("vector-shallow", "(let* ((a (list 9)) (x (vector a 2)) (y (reverse x))) (list (eq a (aref y 1)) (eq x y) (aref x 1)))"),
    ("nil", "(reverse nil)"),
    ("list-copy", "(let* ((x (list 1 2 3)) (y (reverse x))) (list x y (eq x y) (eq (cdr x) (cdr y))))"),
    ("shallow", "(let* ((a (list 9)) (x (list a 2)) (y (reverse x))) (list (eq a (car (cdr y))) x y))"),
    ("singleton", "(let* ((x (list 7)) (y (reverse x))) (list y (eq x y)))"),
    ("dotted", "(condition-case e (reverse '(1 2 . end)) (error e))"),
    ("wrong", "(condition-case e (reverse 17) (error e))"),
    ("symbol", "(condition-case e (reverse 'abc) (error e))"),
    ("cycle", "(let ((x (list 1 2 3))) (setcdr (cdr (cdr x)) x) (condition-case e (reverse x) (circular-list (list (car e) (eq (car (cdr e)) x) (car x) (car (cdr x))))))"),
    ("vector", "(let* ((x (vector 1 2 3)) (y (reverse x))) (list x y (eq x y)))"),
    ("empty-vector", "(let ((x (vector))) (list (reverse x) (eq x (reverse x))))"),
    ("string", "(let* ((x (string 97 98 99)) (y (reverse x))) (list (append x nil) (append y nil) (eq x y)))"),
    ("unicode", "(let ((x (string 955 128578 65))) (list (append (reverse x) nil) (multibyte-string-p (reverse x)) (append x nil)))"),
    ("unibyte", "(let ((x (unibyte-string 0 128 255 65))) (list (append (reverse x) nil) (multibyte-string-p (reverse x))))"),
    ("extended", "(let ((x (string 4194303 1114112 955))) (list (append (reverse x) nil) (multibyte-string-p (reverse x))))"),
    ("raw", "(let ((x (string 4194176 955))) (append (reverse x) nil))"),
    ("multibyte-ascii", "(let ((x (string-to-multibyte \"abc\"))) (list (multibyte-string-p x) (multibyte-string-p (reverse x)) (append (reverse x) nil)))"),
    ("empty-string", "(let ((x (string-to-multibyte \"\"))) (list (append (reverse x) nil) (multibyte-string-p (reverse x)) (eq x (reverse x))))"),
    ("properties", "(let* ((x (string 955 98 99))) (put-text-property 0 3 'face 'bold x) (let ((y (reverse x))) (list (append y nil) (text-properties-at 0 y) (get-text-property 0 'face x))))"),
    ("arity-zero", "(condition-case e (reverse) (wrong-number-of-arguments (list (car e) (car (cdr (cdr e))))))"),
    ("arity-two", "(condition-case e (reverse 1 2) (wrong-number-of-arguments (list (car e) (car (cdr (cdr e))))))"),
]

LIMITATIONS = [
    ("bool-vector", "(reverse (make-bool-vector 3 t))"),
    ("empty-vector-global-identity", "(eq (reverse (vector)) (reverse (vector)))"),
]

if __name__ == "__main__":
    ident = gnu_oracle.require()
    rows = []
    for key, source in CASES + LIMITATIONS:
        expression = "(let ((print-length nil) (print-level nil)) (prin1 " + source + "))"
        result = subprocess.run([ident["path"], "-Q", "--batch", "--eval", expression],
                                stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=30, check=True)
        assert result.stdout, key
        rows.append(dict(id=key, source=source, expected=result.stdout))
    out = Path(__file__).resolve().parents[1] / "tests" / "fixtures" / "reverse.json"
    out.parent.mkdir(exist_ok=True)
    out.write_text(json.dumps(dict(gnu=ident, cases=rows[:len(CASES)], limitations=rows[len(CASES):]), indent=2) + "\n")
    print(f"Recorded {len(CASES)} reverse cases and {len(LIMITATIONS)} limitations to {out}")
