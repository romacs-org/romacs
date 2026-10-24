#!/usr/bin/env python3
"""Augment the from-manifest assoc/delete/nlistp specs with domains and probes.

Run from the repository root:  python3 crates/romacs-vm/tools/primh/logs/assoc/make_spec.py
Each value/probe is Lisp SOURCE evaluated freshly per case on both sides.
Helpers are restricted to functions VM2 already has (checked against the
pre-change binary): no `propertize', `make-bool-vector', `record', `last'
(subr.el), funcall of `eql', and no `#x' reader syntax.
"""
import json
from pathlib import Path

SPECS = Path(__file__).resolve().parents[2] / "specs"


def update(name, **fields):
    p = SPECS / f"{name}.json"
    spec = json.loads(p.read_text())
    spec.update(fields)
    p.write_text(json.dumps(spec) + "\n")
    print(f"{p}: {len(spec.get('probes', []))} probes")


ALIST = ("(list (cons 1 'one) 'junk (cons (copy-sequence \"abc\") 'str) (cons 'sym 's)"
         " (cons 1.5 'f) (cons (list 1 2 3) 'l) (cons nil 'n) (cons (vector 1 2) 'v))")
CYCLIC_ALIST = "(let ((x (list (cons 1 2) (cons 3 4)))) (setcdr (cdr x) x) x)"

# assoc: KEY over the whole catalog; ALIST over the catalog's circular list plus
# alists; TESTFN only over explicit values (tag "testfn" matches no catalog
# entry), because funcall of a non-function is a separate, already-known VM2
# difference (void-function vs invalid-function) and not assoc's behaviour.
update(
    "assoc",
    args=["any", "circular", "testfn"],
    extra={
        "1": ["nil", ALIST, "(list (cons 'a 1) (cons 'b 2))", "(cons (cons 'a 1) 9)",
              "(list 1 2 3)", CYCLIC_ALIST],
        "2": ["nil", "#'eq", "#'equal", "#'="],
    },
    probes=[
        # TESTFN gets (car ELT) then KEY, in list order, once per cons element.
        "(let (calls) (list (assoc 'k (list (cons 'a 1) 7 (cons 'b 2)) (lambda (a k) (push (list a k) calls) nil)) (nreverse calls)))",
        # The matched element itself is returned (identity), first match wins.
        "(let ((p (cons (list 1) 'x))) (let ((r (assoc (list 1) (list p (cons (list 1) 'y))))) (list r (eq r p))))",
        "(let ((p (cons 2 'b))) (eq p (assoc 2 (list (cons 1 'a) p (cons 2 'c)) (lambda (a k) (and (eq a k) 'yes)))))",
        # The tail's cdr is read AFTER the call: TESTFN truncating the list ends the walk.
        "(let ((x (list (cons 1 'a) (cons 2 'b) (cons 3 'c)))) (list (assoc 3 x (lambda (a k) (when (eq a 1) (setcdr x nil)) (eq a k))) x))",
        "(let ((x (list (cons 1 'a) (cons 2 'b)))) (list (assoc 9 x (lambda (a k) (when (eq a 1) (setcdr (cdr x) (list (cons 9 'new)))) (eq a k))) (length x)))",
        # Non-local exits out of TESTFN.
        "(catch 'done (assoc 2 (list (cons 1 'a) (cons 2 'b)) (lambda (a k) (throw 'done (list 'thrown a k)))))",
        "(condition-case e (assoc 1 (list (cons 1 2)) (lambda (a k) (error \"boom %s %s\" a k))) (error e))",
        "(condition-case e (assoc 1 (list (cons 1 2)) 'primh-assoc-no-such-fn) (error e))",
        "(condition-case e (assoc 1 (list (cons 1 2)) (lambda (a) a)) (error (car e)))",
        # A collection inside every callback must not lose the walk's state.
        "(let ((x (list (cons (list 1) 'a) (cons (list 2) 'b) (cons (list 3) 'c)))) (list (assoc (list 3) x (lambda (a k) (let ((i 0)) (while (< i 20000) (cons i i) (setq i (1+ i)))) (equal a k))) x))",
        # A long list through TESTFN (crosses the 4096-step tortoise threshold).
        "(let ((x nil) (i 0)) (while (< i 10000) (setq x (cons (cons i i) x)) (setq i (1+ i))) (assoc 0 x #'eq))",
        "(let* ((end (list (cons 0 0))) (x end) (i 1)) (while (< i 10000) (setq x (cons (cons i i) x)) (setq i (1+ i))) (setcdr end x) (condition-case e (assoc -1 x #'eq) (error (car e))))",
        "(let* ((end (list (cons 0 0))) (x end) (i 1)) (while (< i 10000) (setq x (cons (cons i i) x)) (setq i (1+ i))) (setcdr end (nthcdr 5000 x)) (condition-case e (assoc -1 x) (error (car e))))",
        # Cycles detected by TESTFN-walks and nil-walks alike.
        "(condition-case e (assoc 'z (let ((x (list (cons 'a 1) (cons 'b 2)))) (setcdr (cdr x) x) x) #'eq) (error (car e)))",
        "(condition-case e (assoc 'z (let ((x (list (cons 'a 1) (cons 'b 2)))) (setcdr (cdr x) x) x)) (error (car e)))",
        # equal semantics of the nil TESTFN.
        "(assoc \"b\" (list (cons \"a\" 1) (cons \"b\" 2)))",
        "(assoc 1.0 (list (cons 1 'i) (cons 1.0 'f)))",
        "(assoc 1 (list (cons 1.0 'f) (cons 1 'i)))",
        "(assoc 0.0 (list (cons -0.0 'neg) (cons 0.0 'pos)))",
        "(assoc 0.0e+NaN (list (cons 1 'one) (cons 0.0e+NaN 'nan)))",
        "(assoc (list 1 (vector 2 \"x\")) (list (cons (list 1 (vector 2 \"x\")) 'deep)))",
        "(assoc (vector 1 (list 2)) (list (cons (vector 1 (list 2)) 'v)))",
        # EQ is tried before equal: an eq circular key matches without recursion.
        "(let ((c (list 1 2))) (setcdr (cdr c) c) (cdr (assoc c (list (cons c 'hit)))))",
        # Match before an improper end returns; a miss reaches CHECK_LIST_END.
        "(assoc 'a (cons (cons 'a 1) 9))",
        "(condition-case e (assoc 'z (list (cons 'a 1) (cons 'b 2) 'c)) (error e))",
        "(condition-case e (assoc 'z (cons (cons 'a 1) 9) #'eq) (error e))",
        # Non-conses are skipped without calling TESTFN.
        "(let (seen) (list (assoc 'q (list 1 (cons 'a 1) \"s\" nil (cons 'b 2)) (lambda (a k) (push a seen) nil)) seen))",
        # The result shares structure with ALIST (no copying).
        "(let ((x (list (cons \"k\" (list 1 2))))) (setcar (cdr (assoc \"k\" x)) 'changed) x)",
    ],
)

update(
    "delete",
    args=[],
    extra={
        "0": ["?a", "?\u03bb", "255"],
        "1": ["(list 1 (copy-sequence \"abc\") 1 1.5 (list 1 2 3) 1)",
              "(vector 1 (copy-sequence \"abc\") 1.5 (list 1 2 3) 1)",
              "(copy-sequence \"banana\")", "(copy-sequence \"a\u03bba\u03bb\")",
              "\"a\\377a\"", "(string-to-multibyte \"a\\377a\")",
              "(symbol-function 'car)"],
    },
    probes=[
        # Destructive on lists: the head may move, later cells are spliced.
        "(let ((x (list 1 2 1 3 1))) (let ((saved (cdr x))) (let ((r (delete 1 x))) (list r x (eq r saved) (eq (cdr r) (cdr saved))))))",
        "(let ((x (list 2 1 3 1))) (let ((r (delete 1 x))) (list r x (eq r x))))",
        "(let ((x (list (copy-sequence \"a\") 'b (copy-sequence \"a\")))) (list (delete \"a\" x) x))",
        "(let ((x (list (list 1 2) 3 (list 1 2)))) (list (delete (list 1 2) x) x))",
        "(delete 1 (list 1 1))",
        "(delete 1.0 (list 1 1.0 -0.0 0.0))",
        "(delete 0.0 (list 0.0 -0.0))",
        "(delete 0.0e+NaN (list 0.0e+NaN 1))",
        # A dotted list signals with the (possibly already shortened) SEQ.
        "(condition-case e (delete 1 (cons 1 (cons 1 9))) (error e))",
        "(condition-case e (delete 1 (cons 2 (cons 1 9))) (error e))",
        "(let ((x (list 1 2))) (setcdr (cdr x) x) (condition-case e (delete 3 x) (error (car e))))",
        "(let ((x (list 1 2))) (setcdr (cdr x) x) (condition-case e (length (delete 1 x)) (error (car e))))",
        "(let* ((end (list 0)) (x end) (i 1)) (while (< i 10000) (setq x (cons (% i 3) x)) (setq i (1+ i))) (setcdr end (nthcdr 5000 x)) (condition-case e (delete 7 x) (error (car e))))",
        "(let ((x nil) (i 0)) (while (< i 10000) (setq x (cons (% i 3) x)) (setq i (1+ i))) (length (delete 1 x)))",
        # Vectors: never destructive; same object iff nothing was deleted.
        "(let ((v (vector 1 2))) (eq v (delete 3 v)))",
        "(let ((v (vector 1 2 1))) (let ((r (delete 1 v))) (list r v (eq r v))))",
        "(delete 1 (vector 1 1))",
        "(delete (list 1) (vector (list 1) 2 (list 1)))",
        # Strings: only a character ELT deletes; a new string iff something matched.
        "(let ((s (copy-sequence \"abc\"))) (eq s (delete ?z s)))",
        "(let ((s (copy-sequence \"abca\"))) (let ((r (delete ?a s))) (list r s (eq r s))))",
        "(let ((s (copy-sequence \"abc\"))) (eq s (delete \"a\" s)))",
        "(let ((s (copy-sequence \"abc\"))) (eq s (delete 97.0 s)))",
        "(let ((s (copy-sequence \"a\u03bbb\u03bb\"))) (list (delete ?\u03bb s) (multibyte-string-p (delete ?a s)) (length (delete ?a s))))",
        # Emptied strings keep SEQ's kind (VM2 reads ASCII literals multibyte, VM2-STR1, so be explicit).
        "(let ((s (string-to-multibyte \"aa\")) (u \"\\377\\377\")) (list (delete ?a s) (multibyte-string-p (delete ?a s)) (delete 255 u) (multibyte-string-p (delete 255 u))))",
        "(let ((s \"a\\377b\")) (list (delete 255 s) (delete 4194303 s) (multibyte-string-p (delete 255 s))))",
        "(let ((s (string-to-multibyte \"a\\377b\"))) (list (delete 4194303 s) (delete 255 s) (multibyte-string-p (delete 4194303 s))))",
        "(delete -1 (copy-sequence \"abc\"))",
        "(delete 4194304 (copy-sequence \"abc\"))",
        # Not a sequence.
        "(condition-case e (delete 1 (symbol-function 'car)) (error (car e)))",
        "(condition-case e (delete 1 (make-hash-table)) (error (car e)))",
        # GC pressure after construction, before the call.
        "(let ((x (list (list 1) 2 (list 1) 3))) (let ((i 0)) (while (< i 20000) (cons i i) (setq i (1+ i)))) (list (delete (list 1) x) x))",
    ],
)

update(
    "nlistp",
    args=[],
    extra={"0": ["(symbol-function 'car)", "(make-hash-table)"]},
    probes=[
        "(nlistp (cdr (list 1)))",
        "(list (nlistp nil) (nlistp t) (nlistp (cons 1 2)) (nlistp [1]) (nlistp \"s\") (nlistp 'nil) (nlistp 0))",
        "(let ((x (list 1))) (setcdr x x) (nlistp x))",
    ],
)
