#!/usr/bin/env python3
"""GNU-derived known answers, persisted with exact source and input transport.
No VM expected answer is calculated by this runner. No PMU access required.
"""
import argparse
import json
from pathlib import Path
import subprocess
from bench import GNU, ROOT, read_form

CASES = {
    "lexical-binding": ["lexical-binding", "'lexical-binding"],
    "let*": [
        "(let ((x 1)) (let* ((x 2) (y x)) (+ x y)))",
        "(let* ((x 1) (f (lambda () (setq x (1+ x))))) (funcall f) x)",
        "(let* ((x 1) (x (+ x 2))) x)",
    ],
    "cond": ["(cond)", "(cond (nil 3) ((+ 2 3)))",
        "(let ((x 0)) (cond ((setq x (1+ x))) (t 99)) x)",
        "(cond (nil 1) (t 2 3) (t 4))", "(cond (nil) (nil))"],
    "and": ["(and)", "(and 0 7)", "(let ((x 0)) (and nil (setq x 9)) x)",
        "(and t nil 99)"],
    "or": ["(or)", "(or nil 0 7)", "(let ((x 0)) (or (setq x (1+ x)) (setq x 9)) x)",
        "(let ((\\#:vm2-temp-0 19)) (+ (or nil 4) \\#:vm2-temp-0))"],
    "unless": ["(unless nil 1 2)", "(unless t 1)", "(unless nil)"],
    "when": ["(when t 1 2)", "(when nil 1)", "(when t)"],
    "prog1": ["(let ((x 1)) (prog1 x (setq x 9)))", "(prog1 4)",
        "(let ((x 0)) (+ (prog1 (setq x (1+ x)) (setq x (+ x 10))) x))"],
    "prog2": ["(let ((x 1)) (prog2 (setq x 2) x (setq x 9)))", "(prog2 1 4)"],
    "progn": ["(progn (progn) (progn 2 (progn 3 4)))"],
    "dotimes": ["(let ((x 0)) (dotimes (i 5 x) (setq x (+ x i))))",
        "(dotimes (i 0 i) 99)", "(dotimes (i -3 i) 99)",
        "(let ((n 0)) (dotimes (i (progn (setq n (1+ n)) 4) (+ n i)) (setq n (+ n 10))))",
        "(let ((i 5)) (dotimes (i i i)))",
        "(let ((fs nil)) (dotimes (i 3) (push (lambda () i) fs)) (+ (funcall (car fs)) (* 10 (funcall (car (cdr fs))))))",
        "(let ((x 0)) (dotimes (i 3 x) (setq x (+ x i) i 99)))",
        "(let ((x 0)) (dotimes (i 3 x) (dotimes (j 2) (cl-incf x (+ i j)))))"],
    "dolist": ["(let ((x 0)) (dolist (i (cons 2 (cons 3 nil)) x) (setq x (+ x i))))",
        "(dolist (i nil i) 99)", "(dolist (i (cons 2 nil) i))",
        "(let ((fs nil)) (dolist (i (cons 2 (cons 3 nil))) (push (lambda () i) fs)) (funcall (car fs)))",
        "(let ((i 42)) (dolist (i nil i)))",
        "(let ((i 42)) (dolist (i (cons 2 nil) i) (setq i 99)))"],
    "push": ["(let ((x nil)) (push 2 x) (push 3 x) (+ (car x) (length x)))",
        "(let ((x nil)) (car (push (progn (setq x (cons 1 nil)) 2) x)))"],
    "pop": ["(let ((x (cons 2 (cons 3 nil)))) (+ (pop x) (* 10 (car x))))", "(let (x) (pop x))"],
    "setf": ["(let ((x 1) (y 2)) (setf x (+ x y) y (+ x y)) (+ x y))", "(setf)"],
    "cl-incf": ["(let ((x 2)) (cl-incf x))", "(let ((x 2)) (cl-incf x 7))",
        "(let ((x 2)) (cl-incf x (progn (setq x 100) 7)))"],
    "cl-decf": ["(let ((x 2)) (cl-decf x))", "(let ((x 2)) (cl-decf x 7))"],
    "computed-funcall": ["(funcall (if t (lambda (x) (+ x 3)) (lambda (x) 0)) 4)"],
    "nested-closures": ["(let ((x 2)) (let ((f (let ((y 3)) (lambda (z) (lambda () (+ x y z)))))) (let ((g (funcall f 4))) (setq x 10) (funcall g))))"],
    "returned-lambda": ["(let ((f (let ((x 8)) (lambda () (setq x (1+ x)))))) (+ (funcall f) (funcall f)))"],
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=str(ROOT / "target/debug/romacs-vm"))
    parser.add_argument("--evidence", default=str(ROOT / "evidence/lang-known-answers.json"))
    args = parser.parse_args()
    version = subprocess.check_output([GNU, "-Q", "--batch", "--eval", "(princ emacs-version)"], text=True)
    if version != "32.0.50":
        raise RuntimeError(f"GNU 32.0.50 required, got {version}")
    rows = []
    for form, sources in CASES.items():
        for source in sources:
            data = read_form(source)
            oracle = subprocess.check_output([GNU, "-Q", "--batch", "--eval", "(progn (set-language-environment \"UTF-8\") (set-terminal-coding-system 'utf-8-unix))", "--eval", "(require 'cl-lib)",
                "--eval", "(require 'json)", "--eval", "(setq lexical-binding t)",
                "--eval", f"(princ (decode-coding-string (json-serialize (condition-case e (vector \"value\" (prin1-to-string (eval (read {json.dumps(source, ensure_ascii=False)}) t))) (error (vector \"error\" (symbol-name (car e)))))) 'utf-8))"], text=True)
            kind, expected = json.loads(oracle)
            result = subprocess.run([args.binary], input=data, capture_output=True)
            got = result.stdout.decode().strip()
            ir = subprocess.run([args.binary, "--dump-ir"], input=data, capture_output=True)
            passed = (result.returncode == 0 and got == expected) if kind == "value" else (
                result.returncode != 0 and ((expected == "void-variable" and b"unbound variable" in result.stderr) or (expected == "no-catch" and b"no-catch" in result.stderr)))
            rows.append(dict(form=form, source=source, transport=json.loads(data), expected=expected, kind=kind,
                actual=got, error=result.stderr.decode(), passed=passed,
                ir_ok=ir.returncode == 0, ir=ir.stdout.decode()))
    out = {"gnu": GNU, "version": version, "cases": rows}
    Path(args.evidence).write_text(json.dumps(out, indent=2) + "\n")
    failures = [r for r in rows if not r["passed"] or not r["ir_ok"]]
    for r in failures:
        print(json.dumps(r))
    print(f"GNU {version}: {len(rows) - len(failures)}/{len(rows)} cases; {len(CASES)} form categories")
    raise SystemExit(bool(failures))

if __name__ == "__main__":
    main()
