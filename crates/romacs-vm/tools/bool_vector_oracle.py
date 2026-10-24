#!/usr/bin/env python3
"""Record fresh-process pinned GNU bool-vector results; no Cargo build."""
import os as _portable_os
from pathlib import Path as _PortablePath
import hashlib
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
GNU = Path(_portable_os.environ.get('ROMACS_GNU_ORACLE', str(_PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea/bin/emacs-32.0.50')))
PIN = '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea'
rows = []
def add(name, ident, form):
    rows.append(dict(primitive=name, id=name + '/' + ident, form=form))
def bits(expression):
    return '(let ((r ' + expression + ')) (list (aref r 0) (aref r 1) (aref r 2) (aref r 3)))'

for n in [0, 1, 7, 8, 9, 63, 64, 65]:
    add('make-bool-vector', str(n), f'(bool-vector-count-population (make-bool-vector {n} \'x))')
add('make-bool-vector', 'nil', '(bool-vector-count-population (make-bool-vector 9 nil))')
add('make-bool-vector', 'negative', '(make-bool-vector -1 t)')
add('make-bool-vector', 'type', '(make-bool-vector 1.5 nil)')
add('make-bool-vector', 'shape', '(let ((b (make-bool-vector 9 t))) (list (length b) (type-of b) (arrayp b) (sequencep b) (vectorp b)))')
add('make-bool-vector', 'huge', '(make-bool-vector most-positive-fixnum nil)')
add('bool-vector', 'truth', bits("(bool-vector nil 0 'x t)"))
add('bool-vector', 'empty', '(length (bool-vector))')
add('bool-vector', 'print', '(append (prin1-to-string (bool-vector t nil t)) nil)')
add('bool-vector', 'print-high', '(append (prin1-to-string (make-bool-vector 9 t)) nil)')
add('bool-vector', 'mutation', '(let ((b (bool-vector nil nil))) (list (aset b 1 7) (aref b 0) (aref b 1)))')
add('bool-vector', 'equality', '(list (equal (bool-vector t nil) (bool-vector t nil)) (equal (bool-vector t) (bool-vector nil)))')
for x in ['nil', 't', '[]', '"x"', '(bool-vector)', '(bool-vector nil)']:
    add('bool-vector-p', str(len(rows)), f'(bool-vector-p {x})')
for name in ['bool-vector-exclusive-or', 'bool-vector-union', 'bool-vector-intersection', 'bool-vector-set-difference']:
    a, b = '(bool-vector t t nil nil)', '(bool-vector t nil t nil)'
    add(name, 'truth-table', bits(f'({name} {a} {b})'))
    add(name, 'empty', f'(length ({name} (bool-vector) (bool-vector)))')
    add(name, 'changed-and-stable', f'(let ((a {a}) (b {b}) (d (bool-vector nil nil nil t))) (list (eq ({name} a b d) d) ({name} a b d) (aref d 0) (aref d 1) (aref d 2) (aref d 3)))')
    add(name, 'alias-a', f'(let ((a {a}) (b {b})) ({name} a b a) (list (aref a 0) (aref a 1) (aref a 2) (aref a 3)))')
    add(name, 'alias-b', f'(let ((a {a}) (b {b})) ({name} a b b) (list (aref b 0) (aref b 1) (aref b 2) (aref b 3)))')
    add(name, 'length', f'({name} (bool-vector t) (bool-vector nil nil))')
    add(name, 'dest-length', f'({name} {a} {b} (bool-vector nil))')
    add(name, 'type-a', f'({name} nil {b})')
    add(name, 'type-b', f'({name} {a} nil)')
    add(name, 'type-dest', f'({name} {a} {b} t)')
for x, y in [('nil nil', 'nil nil'), ('t nil', 't t'), ('t t', 't nil'), ('', '')]:
    add('bool-vector-subsetp', str(len(rows)), f'(bool-vector-subsetp (bool-vector {x}) (bool-vector {y}))')
add('bool-vector-subsetp', 'length', '(bool-vector-subsetp (bool-vector) (bool-vector t))')
add('bool-vector-subsetp', 'type', '(bool-vector-subsetp nil (bool-vector))')
add('bool-vector-not', 'truth', bits('(bool-vector-not (bool-vector t nil t nil))'))
add('bool-vector-not', 'padding', '(bool-vector-count-population (bool-vector-not (make-bool-vector 65 nil)))')
add('bool-vector-not', 'dest', '(let ((a (bool-vector t nil)) (d (bool-vector nil nil))) (list (eq (bool-vector-not a d) d) (eq (bool-vector-not a d) d) (aref d 0) (aref d 1)))')
add('bool-vector-not', 'alias', '(let ((a (bool-vector t nil))) (bool-vector-not a a) (list (aref a 0) (aref a 1)))')
add('bool-vector-not', 'length', '(bool-vector-not (bool-vector t) (bool-vector))')
add('bool-vector-not', 'type', '(bool-vector-not nil)')
add('bool-vector-not', 'dest-type', '(bool-vector-not (bool-vector) t)')
for n in [0, 1, 7, 8, 9, 63, 64, 65]:
    add('bool-vector-count-population', str(n), f'(bool-vector-count-population (make-bool-vector {n} t))')
add('bool-vector-count-population', 'type', '(bool-vector-count-population [])')
for val in ['nil', 't', "'truth"]:
    for index in [0, 1, 2, 3, 4]:
        add('bool-vector-count-consecutive', val + str(index), f'(bool-vector-count-consecutive (bool-vector t t nil nil) {val} {index})')
for n in [0, 7, 8, 9, 63, 64, 65]:
    add('bool-vector-count-consecutive', 'boundary'+str(n), f'(bool-vector-count-consecutive (make-bool-vector {n} t) t 0)')
for index in ['-1', '5', 'nil']:
    add('bool-vector-count-consecutive', 'index'+index, f'(bool-vector-count-consecutive (bool-vector t) nil {index})')
add('bool-vector-count-consecutive', 'type', '(bool-vector-count-consecutive nil t 0)')
for ident, form in [
    ('append', '(append (bool-vector t nil) nil)'),
    ('append-final-identity', '(let ((b (bool-vector t nil))) (list (eq b (append b)) (eq b (append nil b)) (eq b (cdr (append (bool-vector t) b)))))'),
    ('append-independent', '(let* ((b (bool-vector t nil)) (r (append b nil))) (aset b 0 nil) (list r (vconcat b)))'),
    ('append-empty', '(append (bool-vector) (bool-vector nil t) nil)'),
    ('nreverse', '(let* ((b (bool-vector t nil)) (r (nreverse b))) (list (eq b r) (vconcat b)))'),
    ('nreverse-alias', '(let* ((b (bool-vector t nil nil)) (r (nreverse b))) (aset r 1 t) (list (vconcat b) (eq b (nreverse r)) (vconcat b)))'),
    ('nreverse-boundary', '(let ((b (make-bool-vector 65 nil))) (aset b 0 t) (aset b 7 t) (nreverse b) (list (length b) (aref b 64) (aref b 57) (aref b 0) (bool-vector-count-population b)))'),
    ('nreverse-empty', '(let ((b (bool-vector))) (list (eq b (nreverse b)) (length b)))'),
    ('fillarray', '(let ((b (bool-vector t nil))) (list (eq b (fillarray b t)) (vconcat b)))'),
    ('fillarray-truth', '(let ((b (make-bool-vector 9 nil))) (list (eq b (fillarray b 0)) (bool-vector-count-population b) (eq b (fillarray b nil)) (bool-vector-count-population b)))'),
    ('fillarray-empty', '(let ((b (bool-vector))) (list (eq b (fillarray b \'x)) (length b)))'),
    ('elt', '(list (elt (bool-vector t nil) 0) (elt (bool-vector t nil) 1))'),
    ('elt-range', '(elt (bool-vector t) 1)'),
    ('reverse', '(vconcat (reverse (bool-vector t nil nil t nil nil nil nil t)))'),
    ('reverse-independent', '(let* ((b (bool-vector t nil)) (r (reverse b))) (aset r 0 t) (list (aref b 1) (aref r 0) (eq b r)))'),
    ('vconcat', '(vconcat [a] (bool-vector t nil) "A")'),
    ('mapc-mutation', '(let ((b (bool-vector t nil)) (seen nil)) (list (eq b (mapc (lambda (v) (setq seen (cons v seen)) (aset b 1 t)) b)) (nreverse seen)))'),
    ('mapcan', '(mapcan (lambda (v) (list v)) (bool-vector nil t))'),
    ('make-vector', '(let* ((b (bool-vector t nil)) (v (make-vector 2 b))) (aset b 1 t) (list (eq (aref v 0) (aref v 1)) (vconcat (aref v 1))))'),
    ('aref-range', '(aref (bool-vector t) 1)'),
    ('aset-range', '(aset (bool-vector t) 1 nil)'),
]:
    add('bool-vector', 'integration-' + ident, form)
for name in sorted({r['primitive'] for r in rows}):
    if name != 'bool-vector':
        add(name, 'arity', f"(apply #'{name} nil)")
        add(name, 'arity-high', f"(apply #'{name} '(nil nil nil nil))")
    add(name, 'callable', f"(list (fboundp '{name}) (functionp #'{name}))")
for row in rows:
    row['source'] = '(prin1-to-string (condition-case e (list \'ok ' + row['form'] + ') (error (list \'sig e))))'
    command = [str(GNU), '--batch', '-Q', '--eval', '(princ ' + row['source'] + ')']
    result = subprocess.run(command, capture_output=True, text=True, check=True, timeout=15)
    row['expected'] = result.stdout
    if row['id'] == 'make-bool-vector/huge':
        row['replay'] = False
        row['reason'] = 'GNU memory-full text is keybinding-substituted and propertized; VM2 inherited plain memory-full fault differs.'
    assert not result.stderr, result.stderr
out = dict(pin=PIN, binary=str(GNU), sha256=hashlib.sha256(GNU.read_bytes()).hexdigest(), rows=rows)
path = ROOT / 'evidence/bool-vector/oracle.json'
path.parent.mkdir(parents=True, exist_ok=True)
path.write_text(json.dumps(out, ensure_ascii=True, indent=2) + '\n')
print(json.dumps({'rows': len(rows), 'primitives': len({r['primitive'] for r in rows}), 'path': str(path)}))
