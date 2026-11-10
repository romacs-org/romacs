#!/usr/bin/env python3
"""Run historical language cases as SOURCE, plus reader/productivity cases.
No Form JSON or inherited reader is involved in VM2's request path.
"""
import argparse
import json
from pathlib import Path
import subprocess
import check_lang

EXTRA = [
    "(list 1 2 'a)", "(member \"x\" '(\"a\" \"x\" \"b\"))", "(nth 2 '(a b c))",
    "(stringp \"λ\")", "(null nil)", "(not t)", "(nconc (list 1 2) (list 3 4))", "(max 1 3.5 2)",
    "(progn (defvar vm2-gate-var 7) (defvar vm2-gate-var (car 1)) vm2-gate-var)",
    "(progn (defconst vm2-gate-constant 9) (let ((vm2-gate-constant 2)) (funcall (lambda () vm2-gate-constant))))",
    "(progn (defalias 'vm2-gate-alias #'(lambda (x) (+ x 2))) (vm2-gate-alias 40))",
    "(progn (put 'vm2-gate-symbol 'vm2-gate-property 17) (get 'vm2-gate-symbol 'vm2-gate-property))",
    "(bare-symbol 'example)",
    "(progn (provide 'vm2-gate-feature) (require 'vm2-gate-feature))",
    "(featurep 'vm2-gate-absent)",

    "(abs -3.14)", "(+ 1 2.5)", "(- 2.5)", "(* 2 1.25)",
    "(/ 7 2)", "(/ 7 2.0)", "(/ 5 2 2.0)", "(/ 1.0 0)",
    "(1+ 2.5)", "(1- 2.5)", "(< 2 2.5 3)", "(= 2 2.0)",
    "1e2", ".5", "-0.0", "1e999", "1.0e+INF", "0.0e+NaN",
    "(= 9007199254740993 9007199254740992.0)",
    "(< 9007199254740992.0 9007199254740993)",
    "(< -9007199254740993 -9007199254740992.0)",
    "(< 1 1.25)", "(< -1.25 -1)", "(= 1 1.0)",
    "(< 1152921504606846975 1.0e+INF)", "(< -1.0e+INF -1152921504606846976)",
    "(= 0.0e+NaN 0)", "(< 0 0.0e+NaN)",
    "(/ 2)", "(/ 2.0)", "(abs -0.0)",
    "(condition-case e (/ 1 0) (error (car e)))",
    "(condition-case e (abs nil) (error e))",
    "(condition-case e (+ 1.0 nil) (error e))",
    "1e-7", "1e-5", "1e14", "1e15", "1000000000000000.5", "1.234567890123456e-5",
    "'nan", "'inf", "'NaN", "'infinity", "(+ 1. 2.)",
    '(+ 1 2)', '; leading comment\n(+ 1 ; inner comment\n 2)',
    '(+ ?λ ?a)', r'(+ ?\n ?\t ?\u03bb)',
    r'(aref "a\n\t\"\\λ" 4)',
    "(cdr '(a . b))", "(car '(a . (b c)))",
    "(length '[a (b . c) 3])", "(aref [a (b . c) 3] 2)",
    "(length ''a)", "(car (cdr '(quote x)))",
    "(length '`(a ,b ,@c))", "(length '(a\\ b \\123))",
    '(funcall #\'(lambda (x) (+ x 2)) 5)',
    '(format "%s:%d:%S:%%" "λ" 7 \'ok)',
    '(process-lines "/usr/bin/printf" "one\\ntwo\\n")',
    '(length (process-lines "/usr/bin/printf" ""))',
    '(process-lines "/usr/bin/printf" "\\n\\n")',
    '(process-lines "/usr/bin/printf" "no-final-newline")',
    '(format "%d" 9 10)',
]

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args()
    sources = [s for group in check_lang.CASES.values() for s in group]
    import check_pending
    sources += list(check_pending.CASES.values())
    import check_s2  # replaces check_lang.CASES with additional cases
    sources += [s for group in check_lang.CASES.values() for s in group]
    sources += EXTRA
    version = subprocess.check_output(['/usr/bin/emacs', '-Q', '--batch', '--eval', '(princ emacs-version)'], text=True)
    assert version == '32.0.50', version
    rows = []
    for source in sources:
        expression = f'''(progn (set-language-environment "UTF-8") (set-terminal-coding-system 'utf-8-unix)
          (require 'cl-lib) (require 'json) (setq lexical-binding t)
          (princ (decode-coding-string (json-serialize (condition-case e
            (vector "value" (prin1-to-string (eval (read {json.dumps(source, ensure_ascii=False)}) t)))
            (error (vector "error" (symbol-name (car e)))))) 'utf-8)))'''
        output = subprocess.check_output(['/usr/bin/emacs', '-Q', '--batch', '--eval', expression], text=True)
        kind, expected = json.loads(output)
        process = subprocess.run([args.binary], input=json.dumps({'source': source}), text=True, capture_output=True)
        actual = process.stdout.strip()
        # ledger/divergences/D001.md. Keep both answers visible and compare
        # against an exact, named expectation, never an ignored mismatch.
        divergence = None
        romacs_expected = expected
        if source == '(condition-case e (let ((s (copy-sequence "αβγ"))) (aset s 1 955) (aref s 1)) (error 18))':
            assert (kind, expected) == ('value', '18')
            divergence = 'ledger/divergences/D001.md'
            romacs_expected = '955'
        if kind == 'value':
            passed = process.returncode == 0 and actual == romacs_expected
        else:
            condition = {'void-variable':'unbound variable', 'no-catch':'(no-catch '}[expected]
            passed = process.returncode == 1 and process.stderr.strip().startswith('vm2: ' + condition)
        rows.append({'source': source, 'gnu_kind': kind, 'gnu': expected, 'romacs_expected': romacs_expected, 'divergence': divergence, 'vm2': actual, 'vm2_error': process.stderr.strip(), 'pass': passed})
    Path(args.output).write_text(json.dumps({'gnu':version,'rows':rows}, ensure_ascii=False, indent=2) + '\n')
    failures = [r for r in rows if not r['pass']]
    print(json.dumps({'cases':len(rows),'passed':sum(r['pass'] for r in rows),'failures':failures}, ensure_ascii=False, indent=2))
    if failures:
        raise SystemExit(1)

if __name__ == '__main__':
    main()
