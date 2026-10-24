#!/usr/bin/env python3
"""Generate focused sequence specs; oracle answers are never synthesized."""
import itertools
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
BASE = HERE / 'primh'
COMMON = dict(effect='global', gnu_files=['fns.c'], args=['leaf-sequences-probes-only'] * 3)
map_probes = [
    '(equal (mapconcat (lambda (x) (list (+ x 64))) \'(1 2 3) "-") "A-B-C")',
    '(equal (funcall #\'mapconcat #\'identity ["a" "b"] \'(955)) "aλb")',
    '(equal (mapconcat #\'list "λ雪" [45]) "λ-雪")',
    '(equal (mapconcat #\'identity \'("a" nil "b")) "ab")',
    '(equal (mapconcat \'invalid nil \'invalid) "")',
    '(multibyte-string-p (mapconcat \'invalid nil \'invalid))',
    '(equal (mapconcat #\'identity \'("a") \'invalid) "a")',
    '(let ((n 0)) (condition-case e (mapconcat (lambda (x) (setq n (1+ n)) x) \'(1 2) "-") (error (list n e))))',
    '(mapconcat \'invalid \'(1 . bad))',
    '(mapconcat #\'identity 1)',
    '(mapconcat #\'identity ["a" "b"] 9)',
    '(mapconcat #\'identity ["a" [99999999]])',
    '(mapconcat \'missing [1])',
    '(let ((x (list "a" "b"))) (setcdr (cdr x) x) (condition-case e (mapconcat #\'identity x) (circular-list (car e))))',
    '(equal (mapconcat (lambda (x) (if x "T" "F")) (make-bool-vector 3 t) ",") "T,T,T")',
    '(mapconcat #\'identity (make-char-table \'test))',
    '(let ((f (symbol-function \'concat))) (fset \'concat (lambda (&rest x) "WRONG")) (equal (mapconcat #\'identity ["a" "b"]) "ab"))',
    '(list (fboundp \'mapconcat) (functionp #\'mapconcat))',
    '(equal (mapconcat #\'identity (vector (unibyte-string 255) (unibyte-string 128)) (unibyte-string 45)) (unibyte-string 255 45 128))',
    '(get-text-property 0 \'face (mapconcat #\'identity (list (propertize "x" \'face \'bold))))',
]
sort_probes = [
    "(sort '(3 1 2) #'<)",
    "(sort nil 'invalid)",
    "(sort '(1) :key 'invalid)",
    "(sort [3 1 2] :lessp #'<)",
    "(sort '((2 . a) (1 . b) (2 . c)) :key #'car :lessp #'< :reverse t)",
    "(sort '((2 . a) (1 . b) (2 . c)) :key #'car :lessp #'<)",
    "(let* ((x (list 3 1 2)) (y (sort x :lessp #'<))) (list x y (eq x y)))",
    "(let* ((x (list 3 1 2)) (tail (cdr x)) (y (sort x #'<))) (list x (eq x y) (eq tail (cdr y))))",
    "(let* ((x (vector 3 1 2)) (y (funcall #'sort x :lessp #'< :in-place t))) (list x (eq x y)))",
    "(let ((x (list 1))) (eq x (sort x)))",
    "(let ((x (vector))) (eq x (sort x)))",
    '(sort "abc")',
    "(sort nil :key #'car :reverse)",
    "(sort nil :bad 1)",
    "(sort nil 1 2)",
    "(sort '(1 . bad) #'<)",
    "(let ((x (list 1 2))) (setcdr (cdr x) x) (condition-case e (sort x #'<) (circular-list (car e))))",
    "(sort '(2 1) :lessp 'missing)",
    "(sort '(2 1) :key 'missing :lessp #'<)",
    "(sort '(3 1 2) :reverse t :reverse nil :lessp #'<)",
    "(let ((n 0)) (list (sort '(3 1 2) :key (lambda (x) (setq n (1+ n)) x) :lessp #'<) n))",
    "(list (fboundp 'sort) (functionp #'sort) (funcall #'sort '(2 1) #'<))",
    "(sort '(3 1 2))",
    "(sort [3 1 2] nil)",
    "(sort '(2 1) :lessp #'< :key #'identity)",
    "(let ((n 0)) (sort '(4 3 2 1) :lessp (lambda (a b) (setq n (1+ n)) (< a b))) n)",
]

# Preserve earlier row identities; append callback traces over every four-item
# permutation in both directions, plus equal-key and one-minrun boundaries.
for values in itertools.permutations(range(1, 5)):
    sequence = "'(" + ' '.join(map(str, values)) + ')'
    for reverse in ['nil', 't']:
        sort_probes.append(f"(let ((trace nil)) (list (sort {sequence} :reverse {reverse} :lessp (lambda (a b) (push (list a b) trace) (< a b))) (nreverse trace)))")
sort_probes.extend([
    "(let ((trace nil)) (sort '(2 2 1 2) :lessp (lambda (a b) (push (list a b) trace) (< a b))) (nreverse trace))",
    "(let ((trace nil)) (sort '((2 . a) (1 . b) (2 . c)) :reverse t :key (lambda (x) (push (cdr x) trace) (car x)) :lessp #'<) (nreverse trace))",
])
for values in [range(63), range(62, -1, -1)]:
    sequence = "'(" + ' '.join(map(str, values)) + ')'
    sort_probes.append(f"(let ((n 0)) (sort {sequence} :lessp (lambda (a b) (setq n (1+ n)) (< a b))) n)")

sort_probes.append('(condition-case nil (progn (sort nil (make-symbol ":reverse") t) \'accepted) (error \'rejected))')

if __name__ == '__main__':
    source = (HERE.parent / 'src/prims/leaf_sequences.rs').read_text().splitlines()
    line = next(i + 1 for i, text in enumerate(source) if 'defsubr! { name = "mapconcat"' in text)
    spec = dict(COMMON, name='mapconcat', arity=[2, 3], gnu_def='fns.c:3484', rust=f'src/prims/leaf_sequences.rs:{line}', probes=map_probes)
    (BASE / 'specs/mapconcat.json').write_text(json.dumps(spec, ensure_ascii=False, indent=2) + '\n')
    line = next(i + 1 for i, text in enumerate(source) if 'defsubr! { name = "sort"' in text)
    spec = dict(COMMON, name='sort', arity=[1, 'many'], gnu_def='fns.c:2447', rust=f'src/prims/leaf_sequences.rs:{line}', probes=sort_probes)
    (BASE / 'specs/sort.json').write_text(json.dumps(spec, ensure_ascii=False, indent=2) + '\n')
