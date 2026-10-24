#!/usr/bin/env python3
"""Fresh-world, compiled-regime oracle for the recovered scalar surface.

The finite domain is deliberately limited to VM2-representable values. Complete
printed GNU condition symbols and data are retained, never dropped or advised.
"""
import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
PROTOCOL = "surface-compiled-v1"
GNU_REVISION = "45b0f7699df36b3849edc3bdef37d4cb9e75b9ea"
VALUES = [
    "nil", "t", "0", "-1", "3.5", "'name", ":keyword", '"λ"',
    "(unibyte-string 255)", "(vector 1)", "(cons 7 9)",
    "(cons (cons 4 nil) nil)", "'(7 9)", '\'("λ")', "(lambda (x) x)", "#'car",
    '(get-buffer-create (generate-new-buffer-name "oracle"))', "(make-marker)",
    "(make-overlay 1 1)",
]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def compiler_id(binary):
    ids = set(re.findall(rb"vm2-source-ir-2:([0-9a-f]{64})", Path(binary).read_bytes()))
    if len(ids) != 1:
        raise RuntimeError("binary must contain exactly one VM2 compiler identity")
    return ids.pop().decode()


def source_digest():
    h = hashlib.sha256()
    for path in sorted((ROOT / "src").rglob("*.rs")):
        data = path.read_bytes()
        h.update(path.relative_to(ROOT).as_posix().encode())
        h.update(len(data).to_bytes(8, "little"))
        h.update(data)
    return h.hexdigest()


def cases():
    for primitive in ["consp", "car-safe", "symbolp"]:
        for value in VALUES:
            yield primitive, f"({primitive} {value})", None
        yield primitive, f"(let ((x (cons 1 nil))) (setcdr x x) ({primitive} x))", None
        # APPLY prevents the compiler replacing an invalid call with its own
        # diagnostic. Preserve the actual subr and argument count in the error.
        for args in ["nil", "'(1 2)"]:
            call = f"(apply #'{primitive} {args})"
            yield primitive, f"(condition-case e {call} (error e))", call
        yield primitive, (
            "(let ((x (cons 7 nil)) (i 0)) "
            "(while (< i 10000) (cons i nil) (setq i (1+ i))) "
            f"({primitive} x))"
        ), None


def reference(gnu, source, home, raw_source):
    # A fresh GNU per candidate VM. No global/function/plist rollback fiction.
    program = r'''
(setq lexical-binding t byte-compile-warnings nil)
(set-language-environment "UTF-8")
(set-terminal-coding-system 'utf-8-unix)
(require 'json)
(defun transport (x)
  (cond ((integerp x) x) ((floatp x) x)
        ((symbolp x) (symbol-name x))
        ((stringp x) (list :string x))
        ((vectorp x) (list :vector (vconcat (mapcar #'transport x))))
        ((consp x) (vconcat (mapcar #'transport x)))
        (t (error "Unsupported transport datum"))))
(let* ((source SOURCE)
       (form (read source))
       (raw nil)
       (fn (byte-compile `(lambda () ,form)))
       (value (condition-case e (funcall fn)
                (error (setq raw (prin1-to-string e)) e))))
  (princ (decode-coding-string (json-serialize
           (list :version emacs-version :revision emacs-repository-version :transport (transport form)
                 :expected (prin1-to-string value) :uncaught raw
                 :condition (when RAW_SOURCE
                   (condition-case e
                     (funcall (byte-compile `(lambda () ,(read RAW_SOURCE))))
                     (error (prin1-to-string e)))))) 'utf-8)))
'''.replace("RAW_SOURCE", "nil" if raw_source is None else json.dumps(raw_source)).replace("SOURCE", json.dumps(source, ensure_ascii=False))
    env = dict(os.environ, HOME=str(home), TMPDIR=str(home),
               XDG_CONFIG_HOME=str(home / "config"),
               XDG_CACHE_HOME=str(home / "cache"),
               XDG_DATA_HOME=str(home / "data"))
    result = subprocess.run([gnu, "-Q", "--batch", "--eval", f"(progn {program})"],
                            capture_output=True, text=True, env=env,
                            check=True, timeout=30)
    row = json.loads(result.stdout)
    if row["revision"] != GNU_REVISION:
        raise RuntimeError(f"GNU source revision differs: {row['revision']}")
    if row["version"] != "32.0.50":
        raise RuntimeError(f"GNU 32.0.50 required: {row['version']}")
    return row


def save(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".pending")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
    temporary.replace(path)


def replay(binary, evidence):
    if evidence["source_sha256"] != source_digest():
        raise RuntimeError("candidate source fingerprint differs from saved oracle")
    if evidence["compiler_id"] != compiler_id(binary):
        raise RuntimeError("binary compiler identity differs from saved oracle")
    failures = []
    for row in evidence["cases"]:
        result = subprocess.run([binary, "--gc-floor", "1"],
                                input=json.dumps(row["transport"]),
                                text=True, capture_output=True, timeout=30)
        actual = result.stdout.strip()
        row["actual"] = actual
        row["stderr"] = result.stderr
        row["passed"] = result.returncode == 0 and actual == row["expected"]
        if not row["passed"]:
            failures.append(row)
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gnu", default="/usr/bin/emacs")
    parser.add_argument("--gnu-source", type=Path)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--replay", action="store_true")
    args = parser.parse_args()
    if args.replay:
        evidence = json.loads(args.evidence.read_text())
    else:
        if args.gnu_source is None:
            parser.error("--gnu-source required when recording")
        source_files = ["data.c", "eval.c", "bytecode.c", "lisp.h", "lread.c", "print.c"]
        for name in source_files:
            pinned = subprocess.check_output(["git", "-C", str(args.gnu_source.parent),
                                              "show", f"{GNU_REVISION}:src/{name}"])
            if pinned != (args.gnu_source / name).read_bytes():
                raise RuntimeError(f"GNU source file differs from executable revision: {name}")
        rows = []
        for primitive, source, raw_source in cases():
            with tempfile.TemporaryDirectory() as home:
                reference_row = reference(args.gnu, source, Path(home), raw_source)
            rows.append(dict(primitive=primitive, source=source, **reference_row))
        assert len(rows) == len(list(cases())) and rows
        evidence = dict(protocol=PROTOCOL, regime="GNU byte-compiled / VM2 compiled",
                        world="fresh process per case on both sides",
                        gc="VM2 collection floor 1; no timing or speedup claim",
                        source_sha256=source_digest(),
                        generator_sha256=digest(__file__),
                        protocol_sha256=hashlib.sha256(PROTOCOL.encode()).hexdigest(),
                        gnu_sha256=digest(args.gnu),
                        gnu_sources={name: digest(args.gnu_source / name)
                                     for name in source_files},
                        compiler_id=compiler_id(args.binary),
                        binary_sha256=digest(args.binary), cases=rows)
        # Persist all reference outcomes before invoking a candidate. A crash
        # cannot silently erase the case which exposed it.
        save(args.evidence, evidence)
    failures = replay(args.binary, evidence)
    if not args.replay:
        save(args.evidence, evidence)
    for row in failures:
        print(json.dumps(row, ensure_ascii=False))
    print(f"{len(evidence['cases']) - len(failures)}/{len(evidence['cases'])} compiled cases")
    raise SystemExit(bool(failures))


if __name__ == "__main__":
    main()
