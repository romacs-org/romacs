#!/usr/bin/env python3
"""Record function's non-lambda identity semantics at the pinned GNU runtime."""
import hashlib, json, pathlib, subprocess, sys
ROOT = pathlib.Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'scripts'))
import gnu_oracle
HERE = pathlib.Path(__file__).resolve().parent
identity = gnu_oracle.require()
forms = ["(list (function 42) (function nil) (function [1 2]) (function (a . b)) (function \"text\"))", "(let ((s (make-symbol \"function-uninterned\"))) (eq s (eval (list 'function s) t)))", "(let ((f (let ((x 42)) (lambda () x)))) (list (eq f (eval (list 'function f) t)) (funcall (eval (list 'function f) t))))"]
rows = []
for form in forms:
    expression = '(princ (json-serialize (prin1-to-string %s)))' % form
    run = subprocess.run([identity['path'], '-Q', '--batch', '--eval', expression], capture_output=True, text=True, check=True, timeout=30)
    rows.append(dict(form=form, gnu=json.loads(run.stdout)))
record = dict(oracle=identity, recorder_sha256=hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(), cases=rows)
(HERE / 'gnu.json').write_text(json.dumps(record, indent=2) + '\n')
print(json.dumps(record, indent=2))
