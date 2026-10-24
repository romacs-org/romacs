#!/usr/bin/env python3
"""Exact-decimal, fresh-process GNU bignum acceptance oracle.

Source is passed to VM2's source compiler, not serde_json numeric transport:
JSON numbers would lose precisely the integers this oracle must test.
GNU byte-compiles the same wrapped source in a fresh process for each case.
Recording requires the pinned GNU executable and source revision. Replay never
starts GNU, including when --gnu names a nonexistent executable. A mismatch
returns nonzero; unsupported syntax/primitive failures are NOT assertion-level
red evidence for later arithmetic or GC operations.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

REVISION = "b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b"
PROTOCOL = "bignums-compiled-source-v1"
SOURCES = ("src/bignum.c", "src/data.c", "src/alloc.c", "src/fns.c",
           "src/lisp.h", "src/print.c", "lisp/subr.el")


def cases():
    rows = []

    def add(family, source):
        rows.append(dict(family=family, source=source))

    # Include VM2's old boundary as well as GNU's real boundary.
    for n in (0, -1, 2**60-1, 2**60, -2**60, -2**60-1,
              2**61-1, 2**61, -2**61, -2**61-1,
              2**63-1, 2**63, 2**64-1, 2**64, 2**64+1,
              10**100+123, -(10**100+123)):
        add("literal", str(n))
        add("predicate", f"(let ((n {n})) (list (integerp n) (fixnump n) (bignump n) (natnump n) (numberp n) (wholenump n)))")
        add("roundtrip", f"(let ((n {n})) (list (number-to-string n) (equal n (read (number-to-string n)))))")
    for form in ("most-positive-fixnum", "most-negative-fixnum",
                 "(1+ 1152921504606846975)", "(1- -1152921504606846976)",
                 "(1+ 2305843009213693951)", "(1- -2305843009213693952)",
                 "(1- 2305843009213693952)", "(1+ -2305843009213693953)",
                 "(- -2305843009213693952)", "(abs -2305843009213693952)",
                 "(* 2305843009213693951 2305843009213693951)",
                 "(+ 18446744073709551616 -18446744073709551616)",
                 "(- 18446744073709551616 18446744073709551615)",
                 "(+ 2305843009213693951 1 -1)"):
        add("arithmetic", form)
    for n in (2**60, 2**61-1, 2**61, -2**61, -2**61-1, 2**100+1):
        # Avoid compiler sharing of equal constant bignums.
        make = f"(f (lambda (x) (+ {n} x)))"
        add("identity", f"(let ({make}) (let ((a (funcall f 0)) (b (funcall f 0))) (list (eq a b) (eql a b) (equal a b))))")
        for test in ("eq", "eql", "equal"):
            add("hash", f"(let ({make} (h (make-hash-table :test '{test}))) (puthash (funcall f 0) 'hit h) (list (gethash (funcall f 0) h) (hash-table-count h)))")
    for x in (2**100+7, -(2**100+7)):
        for y in (3, -3, 2**70+1, -(2**70+1)):
            for op in ("/", "%", "mod"):
                add("division", f"({op} {x} {y})")
    for x in (-7, 7, -(2**100+7), 2**100+7):
        for shift in (-101, -3, 0, 3, 101):
            for op in ("ash", "lsh"):
                add("shift", f"({op} {x} {shift})")
    for form in ("(/ 1 0)", "(/ 1.0 0)", "(/ 1 0.0)", "(% 1 0)",
                 "(mod 1 0)", "(/ 18446744073709551616 0)",
                 "(float 18446744073709551617)",
                 "(float 9007199254740995)",
                 "(float 1" + "0"*400 + ")",
                 "(truncate 1.0e+INF)",
                 "(ash 1 18446744073709551616)",
                 "(ash -7 -18446744073709551616)",
                 "(ash 0 18446744073709551616)",
                 "(let ((integer-width 128)) (ash 1 128))",
                 "(let ((integer-width 128)) (ash 1 127))"):
        add("conversion-error", form)
    for x, y in ((2**100+1, "1.2676506002282294e+30"),
                 (2**53+1, "9007199254740992.0"),
                 (-(2**100+1), "-1.2676506002282294e+30")):
        add("comparison", f"(list (< {x} {y}) (< {y} {x}) (= {x} {y}))")
    sources = [row["source"] for row in rows]
    assert len(sources) == len(set(sources)), "duplicate case"
    return rows


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def wrapped(source):
    return f"(condition-case e {source} (error (list 'signal e)))"


def reference(gnu, source, home):
    program = '''(progn
(setq lexical-binding t byte-compile-warnings nil)
(set-language-environment "UTF-8")
(set-terminal-coding-system 'utf-8-unix)
(require 'json)
(let* ((form (read SOURCE))
       (fn (byte-compile `(lambda () ,form))))
  (princ (json-serialize
    (list :version emacs-version :revision emacs-repository-version
          :expected (prin1-to-string (funcall fn)))))))'''
    program = program.replace("SOURCE", json.dumps(wrapped(source)))
    env = dict(PATH="/usr/bin:/bin", HOME=str(home), TMPDIR=str(home),
               LANG="C.UTF-8", XDG_CONFIG_HOME=str(home / "config"),
               XDG_CACHE_HOME=str(home / "cache"), XDG_DATA_HOME=str(home / "data"))
    result = subprocess.run([gnu, "-Q", "--batch", "--eval", program],
                            env=env, text=True, capture_output=True,
                            check=True, timeout=60)
    row = json.loads(result.stdout)
    if (row["version"], row["revision"]) != ("32.0.50", REVISION):
        raise RuntimeError(f"wrong GNU oracle: {row}")
    return row


def replay(binary, evidence):
    if evidence["generator_sha256"] != sha(__file__):
        raise RuntimeError("generator differs from the recording")
    if [dict(family=r["family"], source=r["source"]) for r in evidence["cases"]] != cases():
        raise RuntimeError("case manifest differs from the recording")
    pinned_binary = sha(binary)
    outcomes = []
    for row in evidence["cases"]:
        result = subprocess.run([binary, "--source", "--gc-floor", "1"],
                                input=wrapped(row["source"]), text=True,
                                capture_output=True, timeout=60)
        actual = result.stdout.rstrip("\n")
        outcomes.append(dict(source=row["source"], family=row["family"],
                             expected=row["expected"], actual=actual,
                             stderr=result.stderr, exit_code=result.returncode,
                             passed=result.returncode == 0 and actual == row["expected"]))
    if sha(binary) != pinned_binary:
        raise RuntimeError("VM2 binary changed while replay was running")
    return outcomes


def save(path, data):
    pending = path.with_suffix(path.suffix + ".pending")
    pending.write_text(json.dumps(data, ensure_ascii=False, indent=1) + "\n")
    pending.replace(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gnu", default="/usr/bin/emacs")
    parser.add_argument("--gnu-repo", type=Path)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--results", type=Path, required=True)
    parser.add_argument("--replay", action="store_true")
    args = parser.parse_args()
    if args.replay:
        evidence = json.loads(args.evidence.read_text())
    else:
        if not args.gnu_repo:
            parser.error("--gnu-repo required to record")
        source_hashes = {}
        for name in SOURCES:
            raw = subprocess.check_output(["git", "-C", str(args.gnu_repo), "show",
                                           f"{REVISION}:{name}"])
            source_hashes[name] = hashlib.sha256(raw).hexdigest()
        rows = []
        for case in cases():
            with tempfile.TemporaryDirectory(prefix="s101-gnu-") as home:
                rows.append(dict(**case, **reference(args.gnu, case["source"], Path(home))))
        evidence = dict(protocol=PROTOCOL,
                        regime="GNU byte-compiled / VM2 source-compiled",
                        world="fresh process per case on both sides",
                        gc="enabled both: GNU default; VM2 floor 1 (not every short case collects)",
                        generator_sha256=sha(__file__), gnu_sha256=sha(args.gnu),
                        gnu_sources=source_hashes, cases=rows)
        save(args.evidence, evidence)
    outcomes = replay(args.binary, evidence)
    passed = sum(row["passed"] for row in outcomes)
    save(args.results, dict(binary_sha256=sha(args.binary), passed=passed,
                            total=len(outcomes), outcomes=outcomes))
    print(f"{passed}/{len(outcomes)} compiled cases; results: {args.results}")
    raise SystemExit(passed != len(outcomes))


if __name__ == "__main__":
    main()
