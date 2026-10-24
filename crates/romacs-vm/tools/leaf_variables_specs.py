#!/usr/bin/env python3
"""Record leaf variable contracts using the native declaration primitive.

primh only admits callable subrs, so C-variable contracts are probes attached
 to internal-make-var-non-special rather than misrepresented as a function.
"""
import json
from pathlib import Path
p=Path(__file__).resolve().parent/'primh/specs/internal-make-var-non-special.json'
p.write_text(json.dumps(dict(name='internal-make-var-non-special',arity=[1,1],effect='global',gnu_files=['eval.c'],gnu_def='eval.c:1065',rust='src/prims/leaf_variables.rs:3',args=['none'],extra={'0':['nil','t',"'leaf-x",'3','"x"','[1]']},probes=[
"(progn (defvar leaf-x 7) (list (special-variable-p 'leaf-x) (internal-make-var-non-special 'leaf-x) (special-variable-p 'leaf-x) leaf-x))",
"(let ((s (make-symbol \"leaf-x\"))) (internal--define-uninitialized-variable s) (list (special-variable-p s) (internal-make-var-non-special s) (special-variable-p s)))",
"(progn (defvaralias 'leaf-alias 'leaf-base) (internal-make-var-non-special 'leaf-alias) (list (special-variable-p 'leaf-alias) (special-variable-p 'leaf-base)))",
"(list (fboundp 'internal-make-var-non-special) (functionp #'internal-make-var-non-special) (funcall #'internal-make-var-non-special 'leaf-y))",
"(list (boundp 'debug-on-error) debug-on-error (special-variable-p 'debug-on-error))",
"(list (let ((debug-on-error '(error))) (funcall (lambda () debug-on-error))) debug-on-error)",
"(progn (setq debug-on-error [1]) (list debug-on-error (default-value 'debug-on-error)))",
"(progn (setq debug-on-error 3) (list debug-on-error (condition-case e (symbol-value 2) (error e))))",
]),indent=2)+'\n')
