#!/usr/bin/env python3
"""Fresh-world, compiled-regime GNU oracle for the regexp engine.

Every case runs in a fresh GNU 32.0.50 process -- a byte-compiled lambda
calling `string-match` under the standard syntax table and the standard case
table -- and in a fresh `regex_probe` process (this crate's
`examples/regex_probe.rs`), which runs the engine alone over the same pattern
and subject with the same tables.  Both print GNU's `(match-data t)` in
character positions, `nil`, or `overflow` for "Stack overflow in regexp
matcher"; the strings are compared whole.

The pathological families are sized so GNU finishes (its time is recorded,
never claimed): GNU's backtracker is exponential on them, so a little larger
and it does not.  Replay gives each romacs case `--timeout` seconds; a
backtracking engine that freezes like GNU fails there.

Recording needs GNU; `--replay` never runs GNU and needs only the evidence:

    tools/regex_oracle.py --binary PROBE --evidence E.json            # record
    tools/regex_oracle.py --binary PROBE --evidence E.json --replay --gnu /nonexistent/gnu
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

PROTOCOL = "regex-string-match-compiled-v1"
GNU_REVISION = "b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b"
GNU_SOURCES = ["regex-emacs.c", "search.c", "syntax.c", "casetab.c"]

A = lambda n: "a" * n  # noqa: E731
LOREM = ("lorem ipsum dolor sit amet, consectetur adipiscing elit; sed do eiusmod "
         "tempor incididunt ut labore et dolore magna aliqua. ") * 200

# (family, pattern, subject, start, case-fold)
CASES = [
    # GNU's empty-loop rule: a capture inside a nullable loop
    ("empty-loop", r"\(a*\)*b", "aab", 0, False),
    ("empty-loop", r"\(a*\)*", "aa", 0, False),
    ("empty-loop", r"\(a*\)*", "", 0, False),
    ("empty-loop", r"\(a*\)*", "b", 0, False),
    ("empty-loop", r"\(a*\)+b", "aab", 0, False),
    ("empty-loop", r"\(a\|\)*b", "aab", 0, False),
    ("empty-loop", r"\(a*\)*\(b\)", "aab", 0, False),
    ("empty-loop", r"\(?:\(a\)\|b*\)*c", "abc", 0, False),
    ("empty-loop", r"\(\(a\)\|\(b\)\)*", "ab", 0, False),
    ("empty-loop", r"\(a*?\)*b", "aab", 0, False),
    ("empty-loop", r"\(a*\)*?b", "aab", 0, False),
    ("empty-loop", r"\(?:\|a\)*\(?:ab\)?", "ab", 0, False),
    ("empty-loop", r"\(?:\|a\)*\(ab\)?", "ab", 0, False),
    ("empty-loop", r"\(x?\)*y", "xxy", 0, False),
    ("empty-loop", r"\(\b\)*a", "a", 0, False),
    ("empty-loop", r"\(?:\(a\)\|\)*b", "aab", 0, False),
    ("empty-loop", r"\(\(a*\)\(b*\)\)*c", "abaabc", 0, False),
    # leftmost-first: alternation priority, greedy and non-greedy
    ("priority", r"a\|ab", "ab", 0, False),
    ("priority", r"ab\|a", "ab", 0, False),
    ("priority", r"\(a\|ab\)\(c\|bcd\)", "abcd", 0, False),
    ("priority", r"\(a\|ab\)\(c\|bcd\)\(d*\)", "abcd", 0, False),
    ("priority", r"a\|ab\|abc", "xabc", 0, False),
    ("priority", r"\(a*\)\(a*\)", "aaa", 0, False),
    ("priority", r"\(a*?\)\(a*\)", "aaa", 0, False),
    ("priority", r"\(.*\)\(.*\)", "abc", 0, False),
    ("priority", r"\(.*?\)\(.*?\)c", "abc", 0, False),
    ("priority", r"\(foo\|foobar\)\(bar\)?", "foobar", 0, False),
    ("non-greedy", r"a+?", "aaa", 0, False),
    ("non-greedy", r"a*?b", "aaab", 0, False),
    ("non-greedy", r"<.*?>", "<a><b>", 0, False),
    ("non-greedy", r"<.*>", "<a><b>", 0, False),
    ("non-greedy", r"\(?:ab\)+?", "ababab", 0, False),
    ("non-greedy", r"x??y", "xy", 0, False),
    ("non-greedy", r"a??", "a", 0, False),
    ("non-greedy", r"\(a\|b\)*?c", "abac", 0, False),
    # the backtracker's own routes: intervals and backreferences
    ("interval", r"a\{2,3\}", "aaaa", 0, False),
    ("interval", r"\(ab\)\{2\}", "xababab", 0, False),
    ("interval", r"[0-9]\{4\}-[0-9]\{2\}", "on 2026-10-18", 0, False),
    ("interval", r"\(a\|b\)\{1,3\}c", "ababc", 0, False),
    ("backref", r"\(a+\)\1", "aaaa", 0, False),
    ("backref", r"\(.\)\1", "abccd", 0, False),
    ("backref", r"\(x?\)*y\1z", "xxyz", 0, False),
    ("backref", r"\(x?\)*y\1z", "xxyxz", 0, False),
    ("backref", r"\<\(\w+\) \1\>", "the the cat", 0, False),
    # zero-width assertions
    ("assertion", r"\bfoo\b", "a foo b", 0, False),
    ("assertion", r"\bfoo\b", "afoob foo", 0, False),
    ("assertion", r"\Boo", "foo", 0, False),
    ("assertion", r"\<bar", "foobar bar", 0, False),
    ("assertion", r"foo\>", "foobar foo", 0, False),
    ("assertion", r"\_<foo-bar\_>", "xfoo-bar foo-bar", 0, False),
    ("assertion", r"\_<a\_>", "a_b a", 0, False),
    ("assertion", r"^b", "a\nb", 0, False),
    ("assertion", r"a$", "a\nb", 0, False),
    ("assertion", r"\`a", "ba", 0, False),
    ("assertion", r"b\'", "bab", 0, False),
    ("assertion", r"\(?:^\|,\)x", "y,x", 0, False),
    # syntax and character classes
    ("class", r"\sw+", "  hello world", 0, False),
    ("class", r"\s-+", "a \t b", 0, False),
    ("class", r"\s_+", "a-_b", 0, False),
    ("class", r"\S-+", "  ab cd", 0, False),
    ("class", r"\(?:\sw\|\s_\)+", "x foo-bar_baz y", 0, False),
    ("class", r"[[:alpha:]]+", "12abc34", 0, False),
    ("class", r"[[:space:]]+", "ab \n c", 0, False),
    ("class", r"[[:upper:]]+", "abCDe", 0, False),
    ("class", r"[[:digit:]]+\.[[:digit:]]+", "v 3.14 x", 0, False),
    ("class", r"[^a-z ]+", "ab CD12 ef", 0, False),
    ("class", r"\W+", "ab--cd", 0, False),
    # case folding against the case table
    ("fold", r"abc", "xABCx", 0, True),
    ("fold", r"[a-z]+", "12HeLLo", 0, True),
    ("fold", r"[[:upper:]]+", "abCDe", 0, True),
    ("fold", r"straße", "STRASSE straße", 0, True),
    ("fold", r"é+", "ÉéÉ", 0, True),
    ("fold", r"\(foo\)\(?:bar\)*", "FOOBARbar", 0, True),
    # multibyte text
    ("multibyte", r"α.*ω", "xαβγωδω", 0, False),
    ("multibyte", r"α.*?ω", "xαβγωδω", 0, False),
    ("multibyte", r"é+", "café ééé", 0, False),
    ("multibyte", r"\w+", "日本語 text", 0, False),
    ("multibyte", r"[α-ω]+", "abc αβγ", 0, False),
    ("multibyte", "\u2018\\(\\w+\\)\u2019", "x \u2018sym\u2019 y", 0, False),
    # start offsets
    ("start", r"a", "aaa", 1, False),
    ("start", r"\(a\)\(b\)?", "xab", 1, False),
    ("start", r"^a", "a\na", 1, False),
    ("start", r"\ba", "aa a", 1, False),
    ("start", r"x*", "abc", 2, False),
    # ordinary patterns in the shapes font-lock and navigation use
    ("ordinary", r"^(def\(?:un\|var\|custom\)\s-+\(\(?:\sw\|\s_\)+\)",
     ";; x\n(defun foo-bar (x)\n", 0, False),
    ("ordinary", r"\_<\(?:let\*?\|when\|unless\)\_>", "(let* ((x 1)) (when x))", 0, False),
    ("ordinary", r"\"\(?:[^\"\\]\|\\.\)*\"", "x \"a\\\"b\" y", 0, False),
    ("ordinary", r";;+ \(.*\)$", "(x) ;; note here\n", 0, False),
    ("ordinary", r"[a-z]+@[a-z]+\.com", LOREM + "mail me@host.com", 0, False),
    ("ordinary", r"[a-z]+@[a-z]+\.org", LOREM, 0, False),
    # pathological, sized for GNU's backtracker
    ("pathological", r"a*a*a*b", A(25), 0, False),
    ("pathological", r"a*a*a*b", A(25) + "!aab", 0, False),
    ("pathological", r"\(a*\)*b", A(20), 0, False),
    ("pathological", r"\(a*\)*b", A(22), 0, False),
    ("pathological", r"\(a*\)*b", A(24), 0, False),
    ("pathological", r"\(a*\)*b", A(24) + "b", 0, False),
    ("pathological", r"\(?:a\|aa\)*b", A(26), 0, False),
    ("pathological", r"\(?:x+x+\)+y", "x" * 23, 0, False),
    ("pathological", r"\(\(a*\)*\)*b", A(14), 0, False),
    ("pathological", r"\(?:a*\)*\(?:a*\)*b", A(15), 0, False),
    ("pathological", r".*.*=.*;", "x" * 400, 0, False),
    ("pathological", r"\(?:\w+\s-*\)+;", "ab " * 9, 0, False),
    # GNU's fail-stack limit: an error both sides
    ("overflow", r"\(?:a\|b\)*c", "ab" * 150000, 0, False),
    ("overflow", r"x\(?:a\|b\)*c", "x" + "ab" * 100000, 0, False),
    ("overflow", r"a*a*a*b", A(400000), 0, False),
    ("overflow", r"x\(?:a\|b\)*c", "x" + "ab" * 10000, 0, False),
]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


GNU_PROGRAM = r'''
(setq byte-compile-warnings nil)
(set-language-environment "UTF-8")
(require 'json)
(let* ((case (with-temp-buffer
               (insert-file-contents CASE-FILE)
               (json-parse-buffer :array-type 'list)))
       (fn (byte-compile
            (lambda (re s start fold)
              (with-syntax-table (standard-syntax-table)
                (let ((case-fold-search fold))
                  (condition-case e
                      (and (string-match re s start) (match-data t))
                    (invalid-regexp 'invalid)
                    (error (if (equal (cadr e) "Stack overflow in regexp matcher")
                               'overflow
                             (list 'error e)))))))))
       (t0 (float-time))
       (result (funcall fn (gethash "re" case) (gethash "s" case)
                        (gethash "start" case) (eq (gethash "fold" case) t))))
  (princ (json-serialize
          (list :version emacs-version :revision emacs-repository-version
                :seconds (- (float-time) t0)
                :expected (prin1-to-string result)))))
'''


def gnu_env(home):
    return dict(PATH="/usr/bin:/bin", LANG="C.UTF-8", HOME=str(home), TMPDIR=str(home),
                XDG_CONFIG_HOME=str(home / "config"), XDG_CACHE_HOME=str(home / "cache"),
                XDG_DATA_HOME=str(home / "data"))


def reference(gnu, case, home):
    case_file = home / "case.json"
    case_file.write_text(json.dumps(case, ensure_ascii=False))
    program = GNU_PROGRAM.replace("CASE-FILE", json.dumps(str(case_file)))
    result = subprocess.run([gnu, "-Q", "--batch", "--eval", f"(progn {program})"],
                            capture_output=True, text=True, env=gnu_env(home), check=True,
                            timeout=600)
    row = json.loads(result.stdout)
    if row["revision"] != GNU_REVISION:
        raise RuntimeError(f"GNU source revision differs: {row['revision']}")
    if row["version"] != "32.0.50":
        raise RuntimeError(f"GNU 32.0.50 required: {row['version']}")
    return row


def probe_line(case):
    hexed = lambda text: text.encode("utf-8").hex()  # noqa: E731
    return "\t".join([hexed(case["re"]), hexed(case["s"]), str(case["start"]),
                      "1" if case["fold"] else "0", "0"]) + "\n"


def subject_of(row):
    """A row's subject: inline when short, else rebuilt from CASES (whose
    generator is pinned by hash) and checked against the recorded digest."""
    if "s" in row:
        return row["s"]
    subject = CASES[row["index"]][2]
    if hashlib.sha256(subject.encode("utf-8")).hexdigest() != row["s_sha256"]:
        raise RuntimeError(f"case {row['index']}: subject differs from the recorded one")
    return subject


def replay(binary, evidence, timeout):
    if evidence["generator_sha256"] != digest(__file__):
        raise RuntimeError("generator differs from the one that recorded the evidence")
    failures = []
    for row in evidence["cases"]:
        row["s"] = subject_of(row)
        case = dict(re=row["re"], s=row["s"], start=row["start"], fold=row["fold"])
        t0 = time.monotonic()
        try:
            result = subprocess.run([binary, "probe"], input=probe_line(case), text=True,
                                    capture_output=True, timeout=timeout)
            row["actual"] = result.stdout.strip()
            row["stderr"] = result.stderr[-2000:]
            ok = result.returncode == 0 and row["actual"] == row["expected"]
        except subprocess.TimeoutExpired:
            row["actual"], row["stderr"], ok = f"timeout after {timeout}s", "", False
        row["romacs_seconds"] = round(time.monotonic() - t0, 3)
        if not ok:
            failures.append(row)
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--gnu", default="/usr/bin/emacs")
    parser.add_argument("--gnu-repo", type=Path, help="GNU git repository containing GNU_REVISION")
    parser.add_argument("--binary", required=True, help="the regex_probe example binary")
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--replay", action="store_true")
    parser.add_argument("--timeout", type=float, default=10.0,
                        help="seconds each romacs case may take on replay")
    args = parser.parse_args()
    if args.replay:
        evidence = json.loads(args.evidence.read_text())
    else:
        if args.gnu_repo is None:
            parser.error("--gnu-repo required when recording")
        gnu_sources = {name: hashlib.sha256(subprocess.check_output(
            ["git", "-C", str(args.gnu_repo), "show", f"{GNU_REVISION}:src/{name}"])).hexdigest()
                       for name in GNU_SOURCES}
        keys = [(p, s, st, f) for _, p, s, st, f in CASES]
        assert len(keys) == len(set(keys)), "duplicate case"
        rows = []
        for index, (family, pattern, subject, start, fold) in enumerate(CASES):
            case = dict(re=pattern, s=subject, start=start, fold=fold)
            with tempfile.TemporaryDirectory() as home:
                row = reference(args.gnu, case, Path(home))
            stored = dict(case)
            if len(subject) > 256:
                del stored["s"]
                stored.update(s_len=len(subject),
                              s_sha256=hashlib.sha256(subject.encode("utf-8")).hexdigest())
            rows.append(dict(index=index, family=family, **stored, expected=row["expected"],
                             gnu_seconds=round(row["seconds"], 4),
                             version=row["version"], revision=row["revision"]))
        evidence = dict(protocol=PROTOCOL,
                        regime="GNU byte-compiled string-match / romacs engine (regex_probe)",
                        world="fresh process per case on both sides; standard syntax and case tables",
                        gc="GNU default GC; romacs engine has no Lisp heap; no timing claim",
                        generator_sha256=digest(__file__), gnu_sha256=digest(args.gnu),
                        gnu_sources=gnu_sources, cases=rows)
        tmp = args.evidence.with_suffix(".pending")
        tmp.write_text(json.dumps(evidence, ensure_ascii=False, indent=1) + "\n")
        evidence = json.loads(tmp.read_text())
        tmp.replace(args.evidence)
    failures = replay(args.binary, evidence, args.timeout)
    for row in failures:
        shown = {k: row[k] for k in ("family", "re", "expected", "actual")}
        shown["s"] = row["s"] if len(row["s"]) <= 60 else f"<{len(row['s'])} chars>"
        print(json.dumps(shown, ensure_ascii=False))
    print(f"{len(evidence['cases']) - len(failures)}/{len(evidence['cases'])} cases "
          f"(binary {digest(args.binary)[:12]})")
    raise SystemExit(bool(failures))


if __name__ == "__main__":
    main()
