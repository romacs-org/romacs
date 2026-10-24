#!/usr/bin/env python3
"""Emit finite scalar specs from the actual declaration file; no oracle answers."""
import json
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
SOURCE = HERE.parents[1] / 'src/prims/leaf_scalars.rs'
OUT = HERE / 'leaf-scalar-specs'
NUMBERS = ['0', '1', '-1', '2', '-2', '0.0', '-0.0', '0.5', '1.25', '-3.75', 'most-positive-fixnum', 'most-negative-fixnum', '1.0e+INF', '-1.0e+INF', '0.0e+NaN', 'nil', 't', "'wrong", '"bad"', "'(1)", '[1]']
PROBES = {
    'point-marker': [
        "(list (fboundp 'point-marker) (functionp #'point-marker) (markerp (funcall #'point-marker)))",
        '(let ((m (point-marker))) (list (markerp m) (marker-position m) (eq (marker-buffer m) (current-buffer)) (marker-insertion-type m) (eq m (point-marker))))',
        '(progn (insert "é🙂z") (goto-char 2) (let ((m (point-marker))) (insert "Q") (list (marker-position m) (point))))',
        '(progn (insert "abc") (goto-char 2) (let ((m (point-marker))) (goto-char 1) (insert "Z") (marker-position m)))',
        '(progn (insert "abc") (goto-char 3) (let ((m (point-marker))) (delete-region 1 3) (marker-position m)))',
        '(progn (insert "abcd") (narrow-to-region 2 4) (goto-char 3) (let ((m (point-marker))) (list (marker-position m) (eq (marker-buffer m) (current-buffer)))))',
        '(progn (set-buffer-multibyte nil) (insert "ab") (goto-char 2) (marker-position (point-marker)))',
        "(progn (fset 'copy-marker (lambda (&rest ignored) nil)) (markerp (point-marker)))",
        '(let ((m (point-marker))) (set-marker m nil) (list (marker-position m) (marker-buffer m)))',
        '(let ((m (point-marker))) (kill-buffer (current-buffer)) (list (marker-position m) (marker-buffer m)))',
    ],
    'atan': ['(atan 0)', '(atan 1)', '(atan -1)', '(atan -0.0)', '(atan 1 nil)', '(atan 1 -1)', '(atan -1 -1)', '(atan 0.0 -0.0)', '(atan -0.0 -0.0)', '(atan (car (read-from-string "#x200000000000000001")))', '(atan 1 (car (read-from-string "#x200000000000000001")))', "(list (fboundp 'atan) (functionp #'atan) (funcall #'atan 0))"],
    'exp': ['(exp -1000)', '(exp 1000)', '(exp 709)', '(= (exp -745) 5e-324)', '(exp (car (read-from-string "#x200000000000000001")))', '(exp (car (read-from-string "#x-200000000000000001")))', "(list (fboundp 'exp) (functionp #'exp) (funcall #'exp 0))"],
    'number-to-string': [f'(number-to-string {x})' for x in ['1e-4', '1e-5', '1e15', '1e16', '1.7976931348623157e308', '5e-324', '(car (read-from-string "#x200000000000000001"))', '(car (read-from-string "#x-200000000000000001"))']] + ['(multibyte-string-p (number-to-string 1.0))', "(list (fboundp 'number-to-string) (functionp #'number-to-string) (append (funcall #'number-to-string 42) nil))"],
}

OUT.mkdir(exist_ok=True)
for line, text in enumerate(SOURCE.read_text().splitlines(), 1):
    m = re.search(r'defsubr! \{ name = "([^"]+)", gnu = "([^"]+)", effect = (\w+)', text)
    if not m:
        continue
    name, gnu, effect = m.groups()
    arity = {'point-marker': [0, 0], 'atan': [1, 2], 'exp': [1, 1], 'number-to-string': [1, 1]}[name]
    spec = dict(name=name, arity=arity, effect=effect.lower(), gnu_files=[gnu.split(':')[0]], gnu_def=gnu, rust=f'src/prims/leaf_scalars.rs:{line}', args=['none'] * arity[1], probes=PROBES[name])
    if arity[1]:
        spec['extra'] = {str(i): NUMBERS for i in range(arity[1])}
    if name in ('atan', 'exp'):
        spec['gnu_files'].append('bignum.c')
    if name == 'atan':
        spec['probes'] += [f'(atan 1 (car (read-from-string "#x{digits}")))' for digits in ['3fffffffffffffffff', 'f' * 256]]
    if name == 'number-to-string':
        spec['gnu_files'].append('print.c')
        # Preserve contents and representation without relying on the inherited
        # #u8 unibyte printer. Errors still retain their full condition/data.
        spec['probes'] = [f'(list (append {p} nil) (multibyte-string-p {p}))' if p.startswith('(number-to-string ') else p for p in PROBES[name]]
        spec['probes'] += [f'(list (append (number-to-string {n}) nil) (multibyte-string-p (number-to-string {n})))' for n in NUMBERS[:15]]
        spec['probes'] += [f'(append (number-to-string (string-to-number "{s}")) nil)' for s in ['7.0e+NaN', '-7.0e+NaN']]
        for fmt in ['%.2f', '%.0f', '%.f', '%.3e', '%.3g', '%.0g', '%.16g', '%f', '%.λ', '%.g', r'%.2f\0junk']:
            for value in ['1.2345', '-0.0', '999.9', '0.00001234']:
                spec['probes'].append(f'(progn (setq float-output-format "{fmt}") (let ((r (append (number-to-string {value}) nil))) (setq float-output-format nil) r))')
        spec['extra'] = {'0': NUMBERS[15:]}
    (OUT / f'{name}.json').write_text(json.dumps(spec, indent=2) + '\n')
