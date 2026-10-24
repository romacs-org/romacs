#!/usr/bin/env python3
"""Source-pinned JSON compatibility preflight, not an implementation claim.

Record GNU outcomes before any candidate replay. Each side gets a fresh world
and compiles the same source. Missing primitives are behavioral failures, not
skips. Representation prerequisite probes are reported separately. Replay never
launches GNU and refuses stale sources/binaries/generators.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

import surface_oracle as surface

ROOT = Path(__file__).resolve().parents[1]
PROTOCOL = "json-native-preflight-v1"


def cases():
    def parse(label, text, options="", projection=None):
        call = f"(json-parse-string {json.dumps(text, ensure_ascii=False)} {options})"
        if projection:
            call = projection.replace("CALL", call)
        return label, call

    for text in ["null", "false", "true", "0", "-1", "1.0", "-0.0", "1e20",
                 "1152921504606846975", "1152921504606846976",
                 "-1152921504606846977", "2305843009213693951", "2305843009213693952",
                 "-2305843009213693952", "-2305843009213693953", "18446744073709551617",
                 "1234567890123456789012345678901234567890", "1e-9999",
                 '"λ😀"', '"\\u0000"', '"\\ud83d\\ude00"',
                 "[]", "[1,2,3]", '[null,false,true,[],{}]']:
        yield parse(f"parse/{text}", text, ":object-type 'alist")
    for object_type in ["alist", "plist"]:
        for text in ['{}', '{"b":1,"a":2,"b":3}',
                     '{"":1,":a":2,"a":3,"nil":4,"t":5}',
                     '{"outer":{"z":1,"x":2},"arr":[1,2]}']:
            yield parse(f"parse/{object_type}/{text}", text,
                        f":object-type '{object_type} :array-type 'list")
    yield parse("parse/hash-default", '{"b":1,"a":2,"b":3}', projection=(
        '(let ((h CALL)) (list (hash-table-p h) (hash-table-test h) '
        '(hash-table-count h) (gethash "a" h) (gethash "b" h)))'))
    yield parse("parse/array-list", "[1,2]", ":array-type 'list")
    yield parse("parse/array-default", "[1,2]")
    yield parse("parse/sentinels", "[null,false]",
                ":null-object 'null-value :false-object 'false-value")
    yield parse("parse/first-keyword-wins", "null",
                ":null-object 'first :null-object 'last")
    for text in ["", " ", "[", '{"a":', "tru", "truex", "00", "1 2",
                 '"\\ud800"', '"\\udc00"', '"\\ud800x"', '"\\u000z"',
                 '"\\x"', '"\x00"', '"a\nb"', "[1,]", '{1:2}',
                 '{"λ":1,\n"a":}', "1e9999"]:
        yield parse(f"error/{text!r}", text)
    for options in [":object-type 'bad", ":array-type 'vector", ":unknown 8",
                    ":null-object", ":object-type 'bad :object-type 'alist"]:
        yield parse(f"keyword/{options}", "null", options)
    yield "parse/wrong-input", "(json-parse-string 1)"
    yield "parse/invalid-utf8", '(json-parse-string (unibyte-string 34 255 34))'
    yield "parse/unibyte-valid-utf8", '(json-parse-string (unibyte-string 34 195 169 34))'
    yield parse("parse/utf8-key-nul", '{"a\\u0000b":1}', ":object-type 'alist")
    yield "roundtrip/bignum", '(json-serialize (json-parse-string "1234567890123456789012345678901234567890"))'
    yield parse("parse/hermes-lossless-contract", '[{},[],null,false]',
                ":null-object :json-null :false-object :json-false", projection=(
                    '(let ((a CALL)) (list (hash-table-p (aref a 0)) '
                    '(length (aref a 1)) (aref a 2) (aref a 3)))'))
    for value in ["nil", "t", ":null", ":false", "1", "1.0", "-0.0", "1e20",
                  '"λ😀"', '(unibyte-string 255)', '[1 t :null :false]',
                  "'(:b 1 :a 2 :b 3)", "'((b . 1) (a . 2) (b . 3))",
                  "'((:a . 1))", "'((\"a\" . 1))", "'(:a)", "'name",
                  "(lambda (x) x)"]:
        yield f"serialize/{value}", f"(json-serialize {value})"
    yield "serialize/float-format-independent", '(let ((float-output-format "%.1f")) (json-serialize 1.234))'
    yield "serialize/null-false-same", "(json-serialize [nil] :null-object nil :false-object nil)"
    yield "serialize/nil-object", "(json-serialize nil :null-object 'other)"
    yield "serialize/plist-cycle", "(let ((x (list :a 1))) (setcdr (cdr x) x) (json-serialize x))"
    yield "serialize/vector-cycle", "(let ((x (vector nil))) (aset x 0 x) (json-serialize x))"
    yield "serialize/alist-recursive", "(let ((x (list (cons 'a nil)))) (setcdr (car x) x) (json-serialize x))"
    yield "serialize/shared-not-cycle", "(let ((x (vector 1))) (json-serialize (vector x x)))"
    yield "serialize/nonstring-hash-key", "(let ((h (make-hash-table))) (puthash 'a 1 h) (json-serialize h))"
    yield "serialize/odd-keywords", "(json-serialize 1 :null-object)"
    yield "serialize/parse-only-keyword", "(json-serialize 1 :array-type 'list)"
    for primitive in ["json-parse-string", "json-serialize"]:
        yield f"arity/{primitive}/zero", f"(apply #'{primitive} nil)"
    yield "buffer/one-value", '(with-temp-buffer (insert "1 2") (goto-char 1) (list (json-parse-buffer) (point)))'
    yield "buffer/error-point", '(with-temp-buffer (insert "[1,") (goto-char 1) (list (condition-case e (json-parse-buffer) (error e)) (point)))'
    yield "buffer/insert", '(with-temp-buffer (json-insert [1 2]) (list (buffer-string) (point)))'
    yield "condition/parse-parent", '(condition-case nil (json-parse-string "1 2") (json-parse-error \'caught) (error \'wrong-parent))'
    yield "condition/json-parent", '(condition-case nil (json-parse-string "\\\"\\\\ud800\\\"") (json-error \'caught) (error \'wrong-parent))'


def prerequisites():
    # Outside the JSON red-first corpus: these identify missing VM substrates.
    return [
        ("fixnum-max", "1152921504606846975"),
        ("fixnum-overflow", "1152921504606846976"),
        ("arbitrary-integer", "1234567890123456789012345678901234567890"),
        ("hash-table", "(make-hash-table :test 'equal)"),
        ("condition-ancestry", "(progn (put 'json-end-of-file 'error-conditions '(json-end-of-file json-parse-error json-error error)) (condition-case nil (signal 'json-end-of-file '(1 nil 0)) (json-parse-error 'caught) (error 'wrong-parent)))"),
    ]


def reference(gnu, source, home):
    # Unlike a serializer projection, printing the outcome retains the complete
    # condition's symbol and data, including GNU's actual subr arity payload.
    program = r'''
(setq s89-end-of-file-before (get 'json-end-of-file 'error-conditions)
      s89-lossless-before (if (fboundp 'json-parse-lossless) t :false))
(require 'json)
(setq lexical-binding t byte-compile-warnings nil)
(set-language-environment "UTF-8")
(set-terminal-coding-system 'utf-8-unix)
(let* ((source SOURCE)
       (form (read source))
       (fn (byte-compile `(lambda () ,form)))
       (print-circle t) (print-length nil) (print-level nil)
       (value (condition-case e (funcall fn) (error e))))
  (princ (decode-coding-string
    (json-serialize
      (list :version emacs-version :revision emacs-repository-version
            :expected (prin1-to-string value)
            :most-positive-fixnum most-positive-fixnum
            :most-negative-fixnum most-negative-fixnum
            :lossless-before s89-lossless-before
            :json-library (symbol-file 'json-read 'defun)
            :eof-ancestry-before (vconcat (mapcar #'symbol-name s89-end-of-file-before))
            :eof-ancestry-after (vconcat (mapcar #'symbol-name (get 'json-end-of-file 'error-conditions)))
            :nonstring-key-ancestry (vconcat (mapcar #'symbol-name (get 'json-object-nonstring-key 'error-conditions)))
            :lossless-bound (if (fboundp 'json-parse-lossless) t :false))) 'utf-8)))
'''.replace("SOURCE", json.dumps(source, ensure_ascii=False))
    env = dict(os.environ, HOME=str(home), TMPDIR=str(home),
               XDG_CONFIG_HOME=str(home / "config"),
               XDG_CACHE_HOME=str(home / "cache"),
               XDG_DATA_HOME=str(home / "data"))
    result = subprocess.run([gnu, "-Q", "--batch", "--eval", f"(progn {program})"],
                            env=env, capture_output=True, text=True,
                            timeout=30, check=True)
    row = json.loads(result.stdout)
    if row["version"] != "32.0.50" or row["revision"] != surface.GNU_REVISION:
        raise RuntimeError(f"GNU identity mismatch: {row}")
    return row


def candidate(binary, source):
    command = [str(binary), "--source", "--gc-floor", "1", "--heap-stats"]
    compiled = subprocess.run([str(binary), "--source", "--dump-ir"],
                              input=source, text=True, capture_output=True, timeout=30)
    if compiled.returncode:
        return dict(compiled=False, compile_error=compiled.stderr.strip(),
                    exit_code=compiled.returncode, actual=None)
    result = subprocess.run(command, input=source, text=True, capture_output=True, timeout=30)
    return dict(compiled=True, exit_code=result.returncode,
                actual=result.stdout.strip(), stderr=result.stderr.strip())


def validate(evidence, binary):
    if evidence["protocol"] != PROTOCOL:
        raise RuntimeError("wrong evidence protocol")
    for field, actual in [
        ("source_sha256", surface.source_digest()),
        ("compiler_id", surface.compiler_id(binary)),
        ("binary_sha256", surface.digest(binary)),
        ("generator_sha256", surface.digest(__file__)),
        ("surface_helper_sha256", surface.digest(surface.__file__)),
    ]:
        if evidence[field] != actual:
            raise RuntimeError(f"stale {field}")
    wanted = list(cases())
    if len(set(label for label, _ in wanted)) != len(wanted):
        raise RuntimeError("duplicate case labels")
    if [(r["id"], r["body"]) for r in evidence["cases"]] != wanted:
        raise RuntimeError("incomplete/changed case corpus")
    if [(r["id"], r["body"]) for r in evidence["prerequisites"]] != prerequisites():
        raise RuntimeError("incomplete/changed prerequisite corpus")


def replay(evidence, binary):
    validate(evidence, binary)
    for group in ["cases", "prerequisites"]:
        for row in evidence[group]:
            actual = candidate(binary, row["source"])
            row.update(actual)
            row["passed"] = (actual["compiled"] and actual["exit_code"] == 0
                             and actual["actual"] == row["expected"])
        rows = evidence[group]
        ok = sum(row["passed"] for row in rows)
        reds = sum(row["compiled"] and not row["passed"] for row in rows)
        invalid = sum(not row["compiled"] for row in rows)
        print(f"{group}: {ok}/{len(rows)} equality; {reds} compiled behavioral failures; {invalid} compile failures")
    invalid = [r["id"] for r in evidence["cases"] if not r["compiled"]]
    if invalid:
        raise RuntimeError(f"JSON red evidence invalid (did not compile): {invalid}")
    # The preflight control MUST run: an unavailable binary is not a red test.
    if evidence["prerequisites"][0]["actual"] != "1152921504606846975":
        raise RuntimeError("known-good fixnum-max candidate control failed")
    return all(row["passed"] for row in evidence["cases"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--gnu", default="/usr/bin/emacs")
    parser.add_argument("--gnu-source", type=Path)
    parser.add_argument("--evidence", required=True, type=Path)
    parser.add_argument("--replay", action="store_true")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if args.replay:
        evidence = json.loads(args.evidence.read_text())
    else:
        if args.gnu_source is None:
            parser.error("recording requires --gnu-source")
        names = ["json.c", "lisp.h", "data.c", "eval.c", "bytecode.c", "print.c"]
        for name in names:
            pinned = subprocess.check_output([
                "git", "-C", str(args.gnu_source.parent), "show",
                f"{surface.GNU_REVISION}:src/{name}"])
            if pinned != (args.gnu_source / name).read_bytes():
                raise RuntimeError(f"GNU source drift: {name}")
        evidence: dict = dict(protocol=PROTOCOL, regime="GNU byte-compiled / VM2 compiled source",
                        world="fresh process per case on both sides",
                        gc="VM2 enabled, floor 1, request-boundary collection; GNU enabled, defaults; no timing claim",
                        status="preflight: implementation blocked on missing VM substrates",
                        source_sha256=surface.source_digest(),
                        compiler_id=surface.compiler_id(binary),
                        binary_sha256=surface.digest(binary),
                        generator_sha256=surface.digest(__file__),
                        surface_helper_sha256=surface.digest(surface.__file__),
                        gnu_sha256=surface.digest(args.gnu),
                        gnu_sources={name: surface.digest(args.gnu_source / name) for name in names})
        for group, inputs in [("cases", cases()), ("prerequisites", prerequisites())]:
            rows = []
            for label, body in inputs:
                source = f"(condition-case err {body} (error err))"
                with tempfile.TemporaryDirectory(prefix="s89-json-") as home:
                    row = reference(args.gnu, source, Path(home))
                rows.append(dict(id=label, body=body, source=source, **row))
            evidence[group] = rows
        libraries = {row["json-library"] for group in ["cases", "prerequisites"]
                     for row in evidence[group]}
        if len(libraries) != 1:
            raise RuntimeError("GNU json library changed during recording")
        evidence["gnu_json_library_sha256"] = surface.digest(libraries.pop())
        json_lisp = args.gnu_source.parent / "lisp/json.el"
        pinned_lisp = subprocess.check_output([
            "git", "-C", str(args.gnu_source.parent), "show",
            f"{surface.GNU_REVISION}:lisp/json.el"])
        if json_lisp.read_bytes() != pinned_lisp:
            raise RuntimeError("GNU json.el source drift")
        evidence["gnu_json_lisp_source_sha256"] = surface.digest(json_lisp)
        # A candidate crash must not erase the reference or its crashing case.
        surface.save(args.evidence, evidence)
    passed = replay(evidence, binary)
    if not args.replay:
        surface.save(args.evidence, evidence)
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
