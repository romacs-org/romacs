#!/usr/bin/env python3
"""Executable GNU-derived acceptance backlog; exits nonzero while any gap remains.
This is intentionally NOT a passing-support claim or a replacement for check_lang.
"""
import json
import subprocess
from pathlib import Path
from bench import GNU, ROOT

CASES = {
    "&optional": "(funcall (lambda (a &optional b) (if b (+ a b) a)) 4)",
    "&rest": "(funcall (lambda (a &rest xs) (+ a (length xs))) 4 5 6)",
    "apply": "(apply '+ 1 (cons 2 (cons 3 nil)))",
    "catch/throw": "(catch 'exit (funcall (lambda () (throw 'exit 42))) 0)",
    "condition-case/signal": "(condition-case e (signal 'error (cons 2 nil)) (error (car (cdr e))))",
    "vector/aref": "(aref [3 7] 1)",
    "vector/aset": "(let ((v (vector 1 2))) (aset v 0 9) (aref v 0))",
    "string/aref": '(aref "abc" 1)',
    "string/aset": '(let ((s (copy-sequence "abc"))) (aset s 1 90) (aref s 1))',
    "assq": "(cdr (assq 'a (cons (cons 'a 7) nil)))",
    "plist-get": "(plist-get (cons 'a (cons 9 nil)) 'a)",
    "generalized-setf": "(let ((x (cons 1 nil))) (setf (car x) 9) (car x))",
    "generalized-cl-incf": "(let ((x (cons 1 nil))) (cl-incf (car x) 9) (car x))",
    "quoted-list": "(car '(4 5))",
}


def main():
    version = subprocess.check_output([GNU, "-Q", "--batch", "--eval", "(princ emacs-version)"], text=True)
    assert version == "32.0.50", version
    rows = []
    for category, source in CASES.items():
        # Reader only. Tagged data records the transport extension required by literals.
        reader = """(progn (require 'json)
          (defun vm2-json (x)
            (cond ((integerp x) x) ((symbolp x) (symbol-name x))
                  ((stringp x) (list :string x))
                  ((vectorp x) (list :vector (vconcat (mapcar #'vm2-json x))))
                  ((consp x) (vconcat (mapcar #'vm2-json x)))
                  (t (error "Unsupported read datum"))))
          (princ (json-serialize (vm2-json (read %s)))))""" % json.dumps(source)
        data = subprocess.check_output([GNU, "-Q", "--batch", "--eval", reader], text=True)
        expected = subprocess.check_output([GNU, "-Q", "--batch", "--eval", "(require 'cl-lib)",
            "--eval", "(setq lexical-binding t)",
            "--eval", f"(prin1 (eval (read {json.dumps(source)}) t))"], text=True).strip()
        result = subprocess.run([str(ROOT / "target/debug/romacs-vm")], input=data, text=True, capture_output=True)
        ir = subprocess.run([str(ROOT / "target/debug/romacs-vm"), "--dump-ir"], input=data, text=True, capture_output=True)
        rows.append(dict(category=category, source=source, transport=json.loads(data), expected=expected,
            actual=result.stdout.strip(), error=result.stderr.strip(), ir=ir.stdout, ir_ok=ir.returncode == 0,
            passed=result.returncode == 0 and result.stdout.strip() == expected and ir.returncode == 0))
    (ROOT / "evidence/lang-pending.json").write_text(json.dumps(dict(gnu=GNU, version=version, cases=rows), indent=2) + "\n")
    for row in rows:
        print(f"{row['category']}: {'PASS' if row['passed'] else 'UNIMPLEMENTED'} GNU={row['expected']} {row['error']}")
    raise SystemExit(any(not row["passed"] for row in rows))

if __name__ == "__main__":
    main()
