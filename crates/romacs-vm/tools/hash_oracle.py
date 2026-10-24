#!/usr/bin/env python3
"""Fresh-world, compiled-regime GNU oracle for VM2 hash tables.

Every case runs in a fresh GNU 32.0.50 process (byte-compiled lexical lambda)
and in a fresh VM2 (compiled transported form, collection floor 1). Both sides
evaluate the identical source, wrapped in the same `condition-case`, so complete
printed conditions are compared, never reduced to their names.

Cases avoid compile-time constant folding on the GNU side: fresh floats and
strings are produced at run time through a let-bound closure or
`copy-sequence`, because GNU's byte compiler shares equal constants and would
turn an `eq` question into a compiler question.

Recording needs GNU; `--replay` never runs GNU and needs only the evidence.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PROTOCOL = "hash-tables-compiled-v1"
GNU_REVISION = "b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b"
GNU_SOURCES = ["fns.c", "lisp.h", "alloc.c", "print.c", "data.c"]
HALF = "(f (lambda (x) (/ x 2)))"  # fresh, unfoldable floats

CASES = [
    # construction and printed representation (GNU 30+ omits size)
    ("print", "(make-hash-table)"),
    ("print", "(make-hash-table :test 'eq)"),
    ("print", "(make-hash-table :test 'equal :size 10)"),
    ("print", "(make-hash-table :weakness 'key)"),
    ("print", "(make-hash-table :weakness 'value :test 'eq)"),
    ("print", "(make-hash-table :weakness 'key-or-value)"),
    ("print", "(make-hash-table :weakness t)"),
    ("print", "(let ((h (make-hash-table :test 'equal))) (puthash \"a\" 1 h) (puthash '(1 2) [x] h) h)"),
    ("print", "(let ((h (make-hash-table))) (puthash nil 'n h) (puthash t 'tt h) (list (gethash nil h) (gethash t h) h))"),
    ("print", "(let ((h (make-hash-table :test 'eq))) (puthash 'a 1 h) (puthash 'a 2 h) (list (hash-table-count h) h))"),
    ("print", "(let ((h (make-hash-table))) (puthash 1 1 h) (puthash 2 2 h) (remhash 1 h) (list h (hash-table-count h)))"),
    # make-hash-table argument parsing and its error order
    ("args", "(make-hash-table :test)"),
    ("args", "(make-hash-table :foo 1)"),
    ("args", "(make-hash-table :foo 1 :bar 2)"),
    ("args", "(make-hash-table :test 'foo)"),
    ("args", "(make-hash-table :size -1)"),
    ("args", "(make-hash-table :size 1.5)"),
    ("args", "(make-hash-table :size 'big)"),
    ("args", "(make-hash-table :weakness 'bad)"),
    ("args", "(make-hash-table :weakness 'bad :test 'bad)"),
    ("args", "(make-hash-table :weakness 'bad :size -1)"),
    ("args", "(progn (put 'half-test 'hash-table-test 'oops) (make-hash-table :test 'half-test))"),
    ("args", "(hash-table-test (make-hash-table :test 'eq :test 'equal))"),
    ("args", "(make-hash-table :rehash-size 2.0 :rehash-threshold 0.5 :purecopy t :test 'eq)"),
    ("args", "(list (hash-table-test (make-hash-table :test nil)) (hash-table-test (make-hash-table :test 'eql)))"),
    ("args", "(hash-table-weakness (make-hash-table :weakness t))"),
    ("args", "(hash-table-weakness (make-hash-table :weakness nil))"),
    ("args", "(list (hash-table-rehash-size (make-hash-table)) (hash-table-rehash-threshold (make-hash-table)))"),
    # type errors carry GNU's predicate and the offending object
    ("type", "(gethash 1 nil)"),
    ("type", "(gethash 1 (vector 1 2))"),
    ("type", "(puthash 1 2 'x)"),
    ("type", "(remhash 1 \"s\")"),
    ("type", "(clrhash (list 1))"),
    ("type", "(hash-table-count 7)"),
    ("type", "(hash-table-size 'h)"),
    ("type", "(hash-table-test nil)"),
    ("type", "(copy-hash-table [h])"),
    ("type", "(maphash (lambda (k v) k) 'not-a-table)"),
    ("type", "(list (hash-table-p (make-hash-table)) (hash-table-p nil) (hash-table-p (vector)) (hash-table-p 'x))"),
    ("type", "(length (make-hash-table))"),
    # subr arity, through apply so the compiler cannot substitute a diagnostic
    ("arity", "(apply #'gethash '(1))"),
    ("arity", "(apply #'gethash '(1 2 3 4))"),
    ("arity", "(apply #'puthash '(1 2))"),
    ("arity", "(apply #'remhash '(1))"),
    ("arity", "(apply #'maphash '(car))"),
    ("arity", "(apply #'hash-table-count nil)"),
    ("arity", "(apply #'clrhash '(1 2))"),
    ("arity", "(apply #'hash-table-p nil)"),
    ("arity", "(apply #'gethash (list 1 (make-hash-table) 'dflt))"),
    ("arity", "(funcall #'puthash 'k 'v (make-hash-table))"),
    # test functions: eq, eql and equal on fresh objects
    ("test", f"(let ({HALF} (h (make-hash-table :test 'eq))) (puthash (funcall f 3.0) 1 h) (list (gethash (funcall f 3.0) h) (hash-table-count h)))"),
    ("test", f"(let ({HALF} (h (make-hash-table :test 'eql))) (puthash (funcall f 3.0) 1 h) (puthash (funcall f 3.0) 2 h) (list (gethash (funcall f 3.0) h) (hash-table-count h)))"),
    ("test", f"(let ({HALF} (h (make-hash-table :test 'equal))) (puthash (funcall f 3.0) 1 h) (gethash (funcall f 3.0) h))"),
    ("test", f"(let ({HALF} (h (make-hash-table))) (puthash (funcall f 0.0) 'pos h) (puthash (funcall f -0.0) 'neg h) (list (gethash (funcall f 0.0) h) (gethash (funcall f -0.0) h) (hash-table-count h)))"),
    ("test", f"(let ({HALF} (h (make-hash-table :test 'equal))) (puthash (funcall f 0.0) 'pos h) (list (gethash (funcall f -0.0) h) (gethash (funcall f 0.0) h)))"),
    ("test", "(let ((nan (lambda () (/ 0.0 0.0))) (h (make-hash-table))) (puthash (funcall nan) 'nan h) (list (gethash (funcall nan) h) (gethash (- (funcall nan)) h) (hash-table-count h)))"),
    ("test", "(let ((nan (lambda () (/ 0.0 0.0))) (h (make-hash-table :test 'equal))) (puthash (- (funcall nan)) 'pos h) (puthash (funcall nan) 'neg h) (list (gethash (funcall nan) h) (gethash (- (funcall nan)) h) (hash-table-count h)))"),
    ("test", "(let ((inf (lambda (x) (/ x 0.0))) (h (make-hash-table :test 'eql))) (puthash (funcall inf 1.0) 'inf h) (puthash (funcall inf -1.0) 'ninf h) (list (gethash (funcall inf 1.0) h) (gethash (funcall inf -1.0) h)))"),
    ("test", "(let ((h (make-hash-table :test 'equal))) (puthash 1 'one h) (list (gethash 1.0 h) (gethash 1 h)))"),
    ("test", "(let ((h (make-hash-table :test 'eq))) (puthash (copy-sequence \"a\") 1 h) (gethash (copy-sequence \"a\") h))"),
    ("test", "(let ((h (make-hash-table :test 'eql))) (puthash (copy-sequence \"a\") 1 h) (gethash (copy-sequence \"a\") h))"),
    ("test", "(let ((h (make-hash-table :test 'equal))) (puthash (copy-sequence \"a\") 1 h) (puthash (copy-sequence \"a\") 2 h) (list (gethash (copy-sequence \"a\") h) (hash-table-count h)))"),
    ("test", "(let ((h (make-hash-table :test 'equal))) (puthash \"λx\" 1 h) (list (gethash (copy-sequence \"λx\") h) (gethash (string-as-unibyte \"λx\") h)))"),
    ("test", "(let ((h (make-hash-table :test 'equal))) (puthash \"abc\" 1 h) (gethash (string-to-unibyte \"abc\") h))"),
    ("test", "(let ((h (make-hash-table :test 'equal))) (puthash (vector 1 (copy-sequence \"a\") (list 2)) 1 h) (list (gethash (vector 1 (copy-sequence \"a\") (list 2)) h) (gethash (vector 1 (copy-sequence \"a\") (list 3)) h)))"),
    ("test", "(let ((h (make-hash-table :test 'equal))) (puthash (list 1 (list 2 (list 3 (list 4 (list 5 6 7 8 9 10))))) 'deep h) (list (gethash (list 1 (list 2 (list 3 (list 4 (list 5 6 7 8 9 10))))) h) (gethash (list 1 (list 2 (list 3 (list 4 (list 5 6 7 8 9 11))))) h)))"),
    ("test", "(let ((h (make-hash-table :test 'equal))) (puthash (cons 1 2) 'dot h) (list (gethash (cons 1 2) h) (gethash (list 1 2) h)))"),
    ("test", "(let ((h (make-hash-table :test 'eq))) (puthash (list 1) 'a h) (gethash (list 1) h))"),
    ("test", "(let ((h (make-hash-table :test 'equal)) (k (list 1 2))) (puthash k 'v h) (setcar k 9) (list (gethash k h) (gethash (list 1 2) h) (gethash (list 9 2) h)))"),
    ("test", "(let ((h (make-hash-table :test 'equal)) (k (list 1))) (setcdr k k) (puthash k 'v h) (gethash k h))"),
    ("test", "(let ((h (make-hash-table :test 'equal)) (k (list 1)) (j (list 1))) (setcdr k k) (setcdr j j) (puthash k 'v h) (gethash j h))"),
    ("test", "(let ((h (make-hash-table :test 'equal)) (k (list 1 2)) (j (list 1))) (setcdr (cdr k) k) (setcdr j j) (puthash k 'v h) (gethash j h))"),
    ("test", "(let ((h (make-hash-table :test 'equal)) (mk (lambda (y) (lambda (x) (+ x y))))) (puthash (funcall mk 1) 1 h) (list (gethash (funcall mk 1) h) (gethash (funcall mk 2) h)))"),
    ("test", "(let ((h (make-hash-table :test 'eq)) (k (make-hash-table))) (puthash k 'table h) (list (gethash k h) (gethash (make-hash-table) h)))"),
    ("test", "(let ((h (make-hash-table :test 'equal))) (puthash :kw 1 h) (puthash 'sym 2 h) (list (gethash :kw h) (gethash 'sym h) (gethash \"sym\" h)))"),
    ("test", "(with-temp-buffer (insert \"abcdef\") (let ((h (make-hash-table :test 'equal)) (m (copy-marker 3))) (puthash m 'm3 h) (list (gethash (copy-marker 3) h) (gethash (copy-marker 4) h) (gethash m h))))"),
    ("test", "(with-temp-buffer (insert \"abcdef\") (let ((h (make-hash-table :test 'eq)) (m (copy-marker 3))) (puthash m 'm3 h) (list (gethash (copy-marker 3) h) (gethash m h))))"),
    # slot model: GNU iteration order is the key_and_value slot order
    ("order", "(let ((h (make-hash-table)) (r nil)) (dolist (k '(c a b e d)) (puthash k (list k) h)) (maphash (lambda (k v) (push (cons k v) r)) h) (nreverse r))"),
    ("order", "(let ((h (make-hash-table))) (puthash 'a 1 h) (puthash 'b 2 h) (puthash 'c 3 h) (remhash 'b h) (puthash 'd 4 h) (list (hash-table-keys h) (hash-table-values h)))"),
    ("order", "(let ((h (make-hash-table))) (dotimes (i 7) (puthash i i h)) (dotimes (i 7) (remhash i h)) (dotimes (i 3) (puthash (+ 10 i) i h)) (list (hash-table-keys h) (hash-table-size h)))"),
    ("order", "(let ((h (make-hash-table))) (puthash 'a 1 h) (puthash 'b 2 h) (remhash 'a h) (remhash 'b h) (clrhash h) (puthash 'x 1 h) (puthash 'y 2 h) (puthash 'z 3 h) (hash-table-keys h))"),
    ("order", "(let ((h (make-hash-table))) (puthash 'a 1 h) (puthash 'b 2 h) (remhash 'a h) (clrhash h) (puthash 'x 1 h) (puthash 'y 2 h) (hash-table-keys h))"),
    ("order", "(let ((h (make-hash-table)) (r nil)) (dotimes (i 200) (puthash i i h) (unless (eq (car r) (hash-table-size h)) (push (hash-table-size h) r))) (nreverse r))"),
    ("order", "(let ((h (make-hash-table :size 3)) (r nil)) (dotimes (i 30) (puthash i i h) (unless (eq (car r) (hash-table-size h)) (push (hash-table-size h) r))) (nreverse r))"),
    ("order", "(let ((h (make-hash-table :size 100)) (r nil)) (dotimes (i 101) (puthash i i h)) (list (hash-table-size h) (hash-table-count h)))"),
    ("order", "(let ((h (make-hash-table :size 0))) (list (hash-table-size h) (progn (puthash 1 1 h) (hash-table-size h)) (progn (clrhash h) (hash-table-size h))))"),
    ("order", "(let ((h (make-hash-table)) (r nil)) (puthash 'a 1 h) (puthash 'b 2 h) (let ((c (copy-hash-table h))) (puthash 'c 3 c) (remhash 'a c) (list (hash-table-keys h) (hash-table-keys c) (hash-table-count h) c)))"),
    # mutation during maphash follows DOHASH_SAFE over the slot model
    ("mutate", "(let ((h (make-hash-table)) (r nil)) (puthash 'a 1 h) (puthash 'b 2 h) (puthash 'c 3 h) (maphash (lambda (k v) (push k r) (when (eq k 'a) (puthash 'z 26 h) (remhash 'c h))) h) (list (nreverse r) h))"),
    ("mutate", "(let ((h (make-hash-table)) (r nil)) (dotimes (i 6) (puthash i i h)) (maphash (lambda (k v) (push k r) (when (< k 100) (puthash (+ k 100) 0 h))) h) (list (length r) (hash-table-size h) (hash-table-count h)))"),
    ("mutate", "(let ((h (make-hash-table)) (r nil)) (dotimes (i 5) (puthash i (* i i) h)) (maphash (lambda (k v) (push v r) (remhash k h)) h) (list (nreverse r) (hash-table-count h) h))"),
    ("mutate", "(let ((h (make-hash-table)) (r nil)) (dotimes (i 5) (puthash i i h)) (maphash (lambda (k v) (push k r) (puthash k (* 10 v) h)) h) (list (nreverse r) (hash-table-values h)))"),
    ("mutate", "(let ((h (make-hash-table)) (r nil)) (dotimes (i 4) (puthash i i h)) (maphash (lambda (k v) (push k r) (when (= k 1) (clrhash h) (puthash 'x 'y h))) h) (list (nreverse r) h))"),
    ("mutate", "(let ((h (make-hash-table)) (r nil)) (dotimes (i 4) (puthash i i h)) (maphash (lambda (k v) (push k r) (when (eq k 0) (remhash 2 h) (puthash 'new 1 h))) h) (nreverse r))"),
    ("mutate", "(let ((h (make-hash-table))) (puthash 1 2 h) (maphash (lambda (k v) (setq k 5)) h))"),
    ("mutate", "(maphash 42 (make-hash-table))"),
    ("mutate", "(let ((h (make-hash-table :test 'equal)) (sum 0)) (dotimes (i 300) (puthash (list i (copy-sequence \"k\")) (vector i) h)) (maphash (lambda (k v) (setq sum (+ sum (aref v 0))) (dotimes (j 20) (cons j k))) h) (list sum (hash-table-count h) (gethash (list 299 \"k\") h)))"),
    # remaining surface
    ("surface", "(list (gethash 1 (make-hash-table) 'dflt) (gethash 1 (make-hash-table)))"),
    ("surface", "(let ((h (make-hash-table))) (list (puthash 'k 'v h) (remhash 'k h) (remhash 'k h) (hash-table-count h)))"),
    ("surface", "(let ((h (make-hash-table))) (puthash 1 2 h) (eq h (clrhash h)))"),
    ("surface", "(let ((h (make-hash-table))) (puthash 'k nil h) (list (hash-table-contains-p 'k h) (hash-table-contains-p 'j h) (gethash 'k h 'missing)))"),
    ("surface", "(let ((h (make-hash-table :test 'equal))) (setf (gethash \"k\" h) 1) (setf (gethash \"k\" h) 2) (list (gethash \"k\" h) (hash-table-count h)))"),
    ("surface", "(let ((h (make-hash-table))) (push 1 (gethash 'k h)) (push 2 (gethash 'k h)) (gethash 'k h))"),
    ("surface", "(let ((h (make-hash-table))) (cl-incf (gethash 'n h 10)) (cl-incf (gethash 'n h 10) 5) (gethash 'n h))"),
    ("surface", "(let ((h (make-hash-table)) (r nil)) (dotimes (i 1000) (puthash i (* 2 i) h)) (dotimes (i 1000) (unless (= (gethash i h) (* 2 i)) (push i r))) (dotimes (i 500) (remhash (* 2 i) h)) (list r (hash-table-count h) (gethash 999 h) (gethash 998 h) (hash-table-size h)))"),
    ("surface", "(let ((h (make-hash-table :test 'equal))) (dotimes (i 200) (puthash (list i (vector i) (copy-sequence \"s\")) i h)) (let ((ok 0)) (dotimes (i 200) (when (eq (gethash (list i (vector i) (copy-sequence \"s\")) h) i) (setq ok (1+ ok)))) ok))"),
    ("surface", "(let ((h (make-hash-table)) (r nil)) (puthash 'a 1 h) (puthash 'b 2 h) (funcall #'maphash (lambda (k v) (push (cons k v) r)) h) r)"),
    ("surface", "(list (apply #'maphash (list #'car (make-hash-table))) (funcall #'maphash #'car (make-hash-table)))"),
    ("surface", "(define-hash-table-test 'vm2-oracle-test #'eq #'sxhash-eq)"),
    ("surface", "#'maphash"),
    ("surface", "(let ((h (make-hash-table))) (puthash 'f (lambda (x) x) h) (funcall (gethash 'f h) 7))"),
]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def wrap(source):
    return f"(condition-case e {source} (error (list 'signal e)))"


def reference(gnu, source, home):
    program = r'''
(setq lexical-binding t byte-compile-warnings nil)
(set-language-environment "UTF-8")
(set-terminal-coding-system 'utf-8-unix)
(require 'json)
(require 'subr-x)
(defun transport (x)
  (cond ((integerp x) x) ((floatp x) x)
        ((symbolp x) (symbol-name x))
        ((stringp x) (list :string x))
        ((vectorp x) (list :vector (vconcat (mapcar #'transport x))))
        ((consp x) (vconcat (mapcar #'transport x)))
        (t (error "Unsupported transport datum"))))
(let* ((form (read SOURCE))
       (fn (byte-compile `(lambda () ,form))))
  (princ (decode-coding-string (json-serialize
           (list :version emacs-version :revision emacs-repository-version
                 :transport (transport form)
                 :expected (prin1-to-string (funcall fn)))) 'utf-8)))
'''.replace("SOURCE", json.dumps(source, ensure_ascii=False))
    env = dict(PATH="/usr/bin:/bin", LANG="C.UTF-8", HOME=str(home), TMPDIR=str(home),
               XDG_CONFIG_HOME=str(home / "config"), XDG_CACHE_HOME=str(home / "cache"),
               XDG_DATA_HOME=str(home / "data"))
    result = subprocess.run([gnu, "-Q", "--batch", "--eval", f"(progn {program})"],
                            capture_output=True, text=True, env=env, check=True, timeout=60)
    row = json.loads(result.stdout)
    if row["revision"] != GNU_REVISION:
        raise RuntimeError(f"GNU source revision differs: {row['revision']}")
    if row["version"] != "32.0.50":
        raise RuntimeError(f"GNU 32.0.50 required: {row['version']}")
    return row


def save(path, value):
    path = Path(path)
    temporary = path.with_suffix(path.suffix + ".pending")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=1) + "\n")
    temporary.replace(path)


def replay(binary, evidence):
    if evidence["generator_sha256"] != digest(__file__):
        raise RuntimeError("generator differs from the one that recorded the evidence")
    failures = []
    for row in evidence["cases"]:
        result = subprocess.run([binary, "--gc-floor", "1"], input=json.dumps(row["transport"]),
                                text=True, capture_output=True, timeout=60)
        row["actual"] = result.stdout.strip()
        row["stderr"] = result.stderr
        if not (result.returncode == 0 and row["actual"] == row["expected"]):
            failures.append(row)
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gnu", default="/usr/bin/emacs")
    parser.add_argument("--gnu-repo", type=Path, help="GNU git repository containing GNU_REVISION")
    parser.add_argument("--binary", required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--replay", action="store_true")
    args = parser.parse_args()
    if args.replay:
        evidence = json.loads(args.evidence.read_text())
    else:
        if args.gnu_repo is None:
            parser.error("--gnu-repo required when recording")
        # The implementation read for this oracle is the executable's own revision.
        gnu_sources = {name: hashlib.sha256(subprocess.check_output(
            ["git", "-C", str(args.gnu_repo), "show", f"{GNU_REVISION}:src/{name}"])).hexdigest()
                       for name in GNU_SOURCES}
        sources = [s for _, s in CASES]
        assert len(sources) == len(set(sources)), "duplicate case"
        rows = []
        for family, source in CASES:
            with tempfile.TemporaryDirectory() as home:
                row = reference(args.gnu, wrap(source), Path(home))
            rows.append(dict(family=family, source=source, **row))
        evidence = dict(protocol=PROTOCOL, regime="GNU byte-compiled / VM2 compiled",
                        world="fresh process per case on both sides",
                        gc="VM2 collection floor 1; GNU default; no timing claim",
                        generator_sha256=digest(__file__),
                        gnu_sha256=digest(args.gnu),
                        gnu_sources=gnu_sources,
                        binary_sha256=digest(args.binary), cases=rows)
        save(args.evidence, evidence)
    failures = replay(args.binary, evidence)
    for row in failures:
        print(json.dumps({k: row[k] for k in ("source", "expected", "actual", "stderr")},
                         ensure_ascii=False))
    print(f"{len(evidence['cases']) - len(failures)}/{len(evidence['cases'])} compiled cases")
    raise SystemExit(bool(failures))


if __name__ == "__main__":
    main()
