#!/usr/bin/env python3
"""Fresh-process byte-compiled GNU / compiled VM2 regexp oracle; offline replay."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
GNU_REVISION = "b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b"


def buf(body, text="ab ab ac"):
    return f'(with-temp-buffer (insert {json.dumps(text)}) (goto-char 1) {body})'


CASES = [
    ("string", '(string-match "b" "abc")'),
    ("string", '(string-match "x" "abc")'),
    ("string", '(string-match "b" "ab ab" 2)'),
    ("string", '(string-match "b" "ab ab" -2)'),
    ("string", '(string-match "" "abc" 3)'),
    ("string", '(string-match "a" "abc" 4)'),
    ("string", '(string-match "a" "abc" -4)'),
    ("string", '(string-match 3 "abc")'),
    ("string", '(string-match "a" 3)'),
    ("string", '(string-match "a" "abc" \'bad)'),
    ("groups", '(progn (string-match "\\\\(a\\\\)\\\\(b\\\\)?\\\\(c\\\\)" "ac") (list (match-beginning 0) (match-end 0) (match-beginning 1) (match-end 1) (match-beginning 2) (match-end 2) (match-string 3 "ac") (match-data)))'),
    ("groups", '(progn (string-match "a\\\\(b\\\\)?" "a") (list (match-string 1 "a") (match-beginning 99) (match-data)))'),
    ("groups", '(progn (string-match "b" "abc") (match-string 0 "xyz"))'),
    ("groups", '(progn (string-match "b" "abc") (match-beginning -1))'),
    ("groups", '(progn (string-match "b" "abc") (match-end \'bad))'),
    ("gc", '(let ((s (copy-sequence "αabcβ"))) (string-match "\\\\(ab\\\\)c" s) (dotimes (i 50) (cons i i)) (list (match-string 0 s) (match-string 1 s) (match-data)))'),
    ("fold", '(let ((case-fold-search t)) (string-match "ÄB" "xäb"))'),
    ("fold", '(let ((case-fold-search nil)) (string-match "ÄB" "xäb"))'),
    ("fold", '(let ((case-fold-search t)) (string-match "[A-Z]+" "abc"))'),
    ("syntax", '(string-match "\\\\bfoo\\\\b" "x foo!")'),
    ("syntax", '(string-match "\\\\_<foo\\\\_>" "foo_bar foo")'),
    ("syntax", '(string-match "\\\\w+\\\\s-+\\\\w+" "foo bar")'),
    ("engine", '(progn (string-match "\\\\(ab\\\\)\\\\1" "zabab") (match-string 0 "zabab"))'),
    ("engine", '(string-match "[" "abc")'),
    ("engine", '(string-match "a\\\\{2,3\\\\}" "aaaa")'),
    ("engine", '(string-match "a.*?b" "a1b2b")'),
    ("unicode", '(progn (string-match "\\\\(β😀\\\\)" "αβ😀x") (list (match-beginning 0) (match-end 0) (match-string 1 "αβ😀x")))'),
    ("inhibit", '(progn (string-match "b" "abc") (string-match "x" "xyz" nil t) (match-data))'),
    ("inhibit", '(progn (string-match "b" "abc") (let ((inhibit-changing-match-data t)) (string-match "x" "xyz")) (match-data))'),
    ("edge", buf('(string-match "b" "abc") (match-string 0)', "XYZ")),
    ("edge", buf('(string-match "a" "abc") (condition-case e (match-string 0) (args-out-of-range (list (car e) (cdr (cdr e)))))', "XYZ")),
    ("looking", buf('(list (looking-at "ab") (point) (match-beginning 0) (match-end 0) (match-string 0))')),
    ("looking", buf('(goto-char 2) (list (looking-at "ab") (point))')),
    ("looking", buf('(goto-char 4) (list (looking-at "\\\\(a\\\\)b") (point) (match-string 1))')),
    ("looking", buf('(looking-at "a") (looking-at "ab" t) (match-end 0)')),
    ("narrow", buf('(narrow-to-region 4 6) (goto-char 4) (list (looking-at "\\\\`ab\\\\\'") (match-beginning 0) (match-end 0) (point))')),
    ("forward", buf('(list (re-search-forward "ab") (point) (match-string 0))')),
    ("forward", buf('(list (re-search-forward "ab" nil nil 2) (point) (match-beginning 0) (match-end 0))')),
    ("backward", buf('(goto-char (point-max)) (list (re-search-backward "ab") (point) (match-string 0))')),
    ("backward", buf('(goto-char (point-max)) (list (re-search-backward "ab" nil nil 2) (point))')),
    ("backward", buf('(goto-char 5) (list (re-search-backward "ab" nil t) (point))')),
    ("backward", buf('(goto-char 4) (list (re-search-backward "ab" nil t) (point))')),
    ("count", buf('(goto-char (point-max)) (list (re-search-forward "ab" nil nil -1) (point))')),
    ("count", buf('(list (re-search-backward "ab" nil nil -1) (point))')),
    ("count", buf('(list (re-search-forward "[" nil nil 0) (point))')),
    ("count", buf('(list (re-search-forward "" nil nil 3) (point))')),
    ("bound", buf('(list (re-search-forward "ab" 2 t) (point))')),
    ("bound", buf('(list (re-search-forward "ab" 3 t) (point))')),
    ("bound", buf('(goto-char 4) (re-search-forward "ab" 2 t)')),
    ("bound", buf('(re-search-backward "ab" 4 t)')),
    ("bound", buf('(goto-char (point-max)) (list (re-search-backward "ab" 5 t) (point))')),
    ("noerror", buf('(list (re-search-forward "z" nil t) (point))')),
    ("noerror", buf('(list (re-search-forward "z" 6 \'move) (point))')),
    ("noerror", buf('(re-search-forward "z")')),
    ("noerror", buf('(goto-char (point-max)) (list (re-search-backward "z" 3 \'move) (point))')),
    ("noerror", buf('(list (re-search-forward "ab" nil t 3) (point) (match-beginning 0))')),
    ("narrow", buf('(narrow-to-region 4 6) (goto-char 4) (list (re-search-forward "ab") (point-min) (point-max) (match-beginning 0))')),
    ("narrow", buf('(narrow-to-region 4 6) (goto-char 6) (list (re-search-backward "ab") (point-min) (point-max))')),
    ("narrow", buf('(narrow-to-region 4 6) (goto-char 4) (list (re-search-forward "ac" nil t) (point))')),
    ("fold", buf('(let ((case-fold-search nil)) (list (looking-at "AB") (re-search-forward "AB" nil t)))')),
    ("gc", buf('(looking-at "\\\\(a\\\\)b") (dotimes (i 50) (cons i i)) (list (match-string 0) (match-string 1) (point))')),
    ('syntax', '(with-temp-buffer (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 45 "w") (modify-syntax-entry 95 "w") (string-match "\\\\w+" "foo-bar _"))'),
    ('syntax', '(with-temp-buffer (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 45 "w") (modify-syntax-entry 95 "w") (string-match "\\\\b_\\\\b" "foo-bar _"))'),
    ('syntax', '(with-temp-buffer (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 45 "w") (modify-syntax-entry 95 "w") (string-match "\\\\s_" "foo-bar _"))'),
    ('syntax', '(with-temp-buffer (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 45 "w") (modify-syntax-entry 95 "w") (string-match "\\\\_<foo-bar\\\\_>" "foo-bar _"))'),
    ('syntax', '(with-temp-buffer (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 45 "w") (modify-syntax-entry 95 "w") (string-match "[[:word:]]+" "foo-bar _"))'),
    ('syntax', '(with-temp-buffer (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 45 "w") (insert "foo-bar") (goto-char 1) (list (looking-at "\\\\w+") (match-string 0) (re-search-forward "\\\\w+")))'),
    ('syntax', '(let ((a (generate-new-buffer "a")) (b (generate-new-buffer "b"))) (set-buffer a) (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 45 "w") (set-buffer b) (list (string-match "\\\\w+" "-") (progn (set-buffer a) (string-match "\\\\w+" "-"))))'),
    ('syntax', '(with-temp-buffer (insert "a-b") (put-text-property 2 3 \'syntax-table \'(2)) (goto-char 1) (let ((parse-sexp-lookup-properties t)) (looking-at "\\\\w+") (let ((s (match-string 0))) (list (length s) (get-text-property 1 \'syntax-table s)))))'),
    ('groups', '(with-temp-buffer (insert "abc") (goto-char 1) (looking-at "\\\\(ab\\\\)c") (let ((m (match-data))) (list (marker-position (car m)) (marker-position (car (cdr m))) (eq (marker-buffer (car m)) (current-buffer)))))'),
    ('groups', '(with-temp-buffer (insert "abc") (goto-char 1) (looking-at "a") (let ((m (match-data t))) (list (car m) (car (cdr m)) (eq (car (cdr (cdr m))) (current-buffer)))))'),
    ('groups', '(progn (string-match "b" "abc") (let ((r (list 8 8 8 8))) (list (eq r (match-data t r)) r)))'),
    ('groups', '(progn (string-match "\\\\(b\\\\)" "abc") (let ((r (list 8))) (list (eq r (match-data t r)) r)))'),
    ('edge', '(with-temp-buffer (insert "XYZ") (goto-char 1) (looking-at "XY") (string-match "b" "abc") (list (match-beginning 0) (match-string 0) (match-string 0 "abc")))'),
    ('bound', '(with-temp-buffer (insert "a\nb") (goto-char 1) (list (re-search-forward "a$" 2 t) (point)))'),
    ('bound', '(with-temp-buffer (insert "ab") (goto-char 1) (list (re-search-forward "a$" 2 t) (point)))'),
    ('engine', '(progn (string-match "b" "abc") (apply #\'string-match \'("a")))'),
    ('engine', "(apply #'looking-at nil)"),
    ('engine', '(apply #\'match-string \'(0 "abc" 1))'),
    ('engine', '(progn (string-match "b" "abc") (match-string 0 3))'),
    ('engine', '(progn (string-match "b" "abc") (match-string 0 ""))'),
    ('unicode', '(progn (string-match "a" (unibyte-string 255 97 0)) (list (match-beginning 0) (match-end 0) (aref (match-string 0 (unibyte-string 255 97 0)) 0)))'),
    ('unicode', '(with-temp-buffer (set-buffer-multibyte nil) (insert (unibyte-string 255 97 0)) (goto-char 1) (list (re-search-forward "a") (match-beginning 0) (aref (match-string 0) 0)))'),
    ('bound', '(with-temp-buffer (insert "abc") (goto-char 1) (re-search-forward "b" (copy-marker 3)))'),
    ('bound', '(with-temp-buffer (insert "abc") (goto-char 1) (re-search-forward "b" \'bad))'),
    ('bound', '(with-temp-buffer (insert "abc") (goto-char 1) (re-search-forward "b" (make-marker)))'),
    ('groups', '(progn (string-match "b" "abc") (match-string 0 [1 2 3]))'),
    ('syntax', '(with-temp-buffer (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 45 "w") (dotimes (i 100) (copy-sequence "dead")) (string-match "\\\\w" "-"))'),
    ('groups', '(with-temp-buffer (insert "abc") (goto-char 1) (looking-at "a") (let* ((r (match-data)) (m (car r))) (string-match "b" "abc") (match-data t r t) (list (marker-position m) r)))'),
    ('syntax', '(with-temp-buffer (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 45 "w") (string-match "[[:word:]]+" "-"))'),
    ('gc', '(progn (set-buffer (generate-new-buffer "source")) (insert "abc") (goto-char 1) (looking-at "a") (set-buffer (generate-new-buffer "other")) (dotimes (i 100) (copy-sequence "dead")) (let ((m (match-data))) (list (marker-position (car m)) (eq (marker-buffer (car m)) (current-buffer)))))'),
    ('engine', '(string-match "\\\\=" "abc" 2)'),
    ('engine', '(with-temp-buffer (insert "abcdef") (goto-char 4) (string-match "\\\\=" "abc"))'),
    ('engine', '(with-temp-buffer (insert "abcdef") (goto-char 4) (string-match "\\\\=" "abc" 2))'),
    ('syntax', '(with-temp-buffer (insert "a-b") (let ((table (copy-syntax-table))) (modify-syntax-entry 45 "w" table) (put-text-property 2 3 \'syntax-table table)) (goto-char 1) (let ((parse-sexp-lookup-properties t)) (looking-at "\\\\w+") (length (match-string 0))))'),
    ('syntax', '(with-temp-buffer (let ((s (copy-sequence "a-b"))) (put-text-property 1 2 \'syntax-table \'(2) s) (let ((parse-sexp-lookup-properties t)) (string-match "\\\\w+" s) (length (match-string 0 s)))))'),
    ('syntax', '(with-temp-buffer (let ((s (copy-sequence "a-b")) (table (copy-syntax-table))) (modify-syntax-entry 45 "w" table) (put-text-property 1 2 \'syntax-table table s) (let ((parse-sexp-lookup-properties t)) (string-match "\\\\w+" s) (length (match-string 0 s)))))'),
    ('syntax', '(with-temp-buffer (set-syntax-table (copy-syntax-table)) (modify-syntax-entry 45 "w") (insert "a-b") (put-text-property 2 3 \'syntax-table (copy-syntax-table (standard-syntax-table))) (goto-char 1) (let ((parse-sexp-lookup-properties t)) (looking-at "\\\\w+") (length (match-string 0))))'),
    ('syntax', '(with-temp-buffer (let ((s (copy-sequence "a-b"))) (put-text-property 1 2 \'syntax-table \'(2) s) (let ((parse-sexp-lookup-properties nil)) (string-match "\\\\w+" s) (length (match-string 0 s)))))'),
    ('syntax', '(with-temp-buffer (insert "-") (put-text-property 1 2 \'syntax-table \'(2)) (goto-char 1) (let ((parse-sexp-lookup-properties t)) (re-search-forward "[[:word:]]+" nil t)))'),
    ('syntax', '(with-temp-buffer (set-buffer-multibyte nil) (insert "a-b") (put-text-property 2 3 \'syntax-table \'(2)) (goto-char 1) (let ((parse-sexp-lookup-properties t)) (looking-at "\\\\w+") (length (match-string 0))))'),
]
# Patterns above are represented as Lisp source: reduce Python's explicit
# doubled escape spelling to exactly the two backslashes Lisp reads as one.
CASES = [(family, source.replace("\\\\\\\\", "\\\\")) for family, source in CASES]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def reference(gnu, source, home):
    program = r'''
(setq lexical-binding t byte-compile-warnings nil)
(set-language-environment "UTF-8")
(require 'json)
(defun transport (x)
  (cond ((integerp x) x) ((floatp x) x)
        ((symbolp x) (symbol-name x))
        ((stringp x) (list :string x))
        ((vectorp x) (list :vector (vconcat (mapcar #'transport x))))
        ((consp x) (vconcat (mapcar #'transport x)))
        (t (error "Unsupported transport datum"))))
(let* ((form (read SOURCE)) (fn (byte-compile `(lambda () ,form))))
  (princ (decode-coding-string (json-serialize
    (list :version emacs-version :revision emacs-repository-version
          :transport (transport form) :expected (prin1-to-string (funcall fn)))) 'utf-8)))
'''.replace("SOURCE", json.dumps(f"(condition-case e {source} (error (list 'signal e)))", ensure_ascii=False))
    env = dict(PATH="/usr/bin:/bin", LANG="C.UTF-8", HOME=str(home), TMPDIR=str(home),
               XDG_CONFIG_HOME=str(home / "config"), XDG_CACHE_HOME=str(home / "cache"),
               XDG_DATA_HOME=str(home / "data"))
    result = subprocess.run([gnu, "-Q", "--batch", "--eval", f"(progn {program})"],
                            capture_output=True, text=True, env=env, check=True, timeout=60)
    row = json.loads(result.stdout)
    assert row["revision"] == GNU_REVISION and row["version"] == "32.0.50", row
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gnu", default="/usr/bin/emacs")
    parser.add_argument("--gnu-repo", type=Path)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--replay", action="store_true")
    parser.add_argument("--scratch", type=Path, help="explicit temporary directory for live GNU cases")
    args = parser.parse_args()
    if args.replay:
        evidence = json.loads(args.evidence.read_text())
        assert evidence["generator_sha256"] == digest(__file__)
    else:
        assert args.gnu_repo is not None, "--gnu-repo required when recording"
        assert args.scratch is not None, "--scratch required when recording"
        sources = {name: hashlib.sha256(subprocess.check_output(
            ["git", "-C", str(args.gnu_repo), "show", f"{GNU_REVISION}:src/{name}"])).hexdigest()
                   for name in ["search.c", "regex-emacs.c"]}
        rows = []
        assert len(CASES) == len(set(source for _, source in CASES))
        for family, source in CASES:
            with tempfile.TemporaryDirectory(dir=args.scratch) as home:
                row = reference(args.gnu, source, Path(home))
            rows.append(dict(family=family, source=source, **row))
        evidence = dict(protocol="vm2-regexp-compiled-v1", generator_sha256=digest(__file__),
                        regime="fresh process each case; GNU byte-compiled; VM2 compiled",
                        gc="VM2 floor 1; GNU default; no timing claim", gnu_sources=sources,
                        gnu_sha256=digest(args.gnu), cases=rows)
        args.evidence.write_text(json.dumps(evidence, ensure_ascii=False, indent=1) + "\n")
    failures = 0
    for row in evidence["cases"]:
        result = subprocess.run([args.binary, "--gc-floor", "1"],
                                input=json.dumps(row["transport"]), capture_output=True,
                                text=True, timeout=60)
        actual = result.stdout.strip()
        if result.returncode or actual != row["expected"]:
            failures += 1
            print(json.dumps(dict(source=row["source"], expected=row["expected"],
                                  actual=actual, stderr=result.stderr), ensure_ascii=False))
    print(f"{len(evidence['cases']) - failures}/{len(evidence['cases'])} compiled cases")
    raise SystemExit(bool(failures))


if __name__ == "__main__":
    main()
