#!/usr/bin/env python3
"""Record fresh-world compiled GNU control answers; replay without GNU."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
from surface_oracle import compiler_id, source_digest

ROOT = Path(__file__).resolve().parents[1]
REVISION = "b28750b822dc45ffdceda3ea44a3c8b93f4e5a6b"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def save(path, data):
    pending = path.with_suffix(path.suffix + ".pending")
    pending.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n")
    pending.replace(path)


def gnu_call(binary, program):
    with tempfile.TemporaryDirectory(prefix="control-oracle-") as directory:
        env = dict(os.environ, HOME=directory, TMPDIR=directory,
                   XDG_CONFIG_HOME=directory, XDG_CACHE_HOME=directory,
                   XDG_DATA_HOME=directory, XDG_STATE_HOME=directory)
        program = "(progn (set-terminal-coding-system 'utf-8-unix) " + program + ")"
        result = subprocess.run([binary, "-Q", "--batch", "--eval", program],
                                text=True, capture_output=True, env=env,
                                timeout=30, check=True)
        return json.loads(result.stdout)


def record(binary, source):
    # Native json-serialize needs no (require 'json). Requiring that library
    # before measurement changes json-end-of-file ancestry.
    program = r'''(progn
      (setq lexical-binding t byte-compile-warnings nil)
      (let* ((fn (byte-compile `(lambda () ,(read SOURCE))))
             (answer (funcall fn)))
        (princ (decode-coding-string
                (json-serialize (vector emacs-version emacs-repository-version
                                       (prin1-to-string answer))) 'utf-8))))'''
    version, revision, answer = gnu_call(binary, program.replace("SOURCE", json.dumps(source)))
    if (version, revision) != ("32.0.50", REVISION):
        raise RuntimeError("GNU executable revision differs")
    return answer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gnu", default="/usr/bin/emacs")
    parser.add_argument("--gnu-repository", type=Path)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--record", action="store_true")
    args = parser.parse_args()
    paths = [ROOT / "evidence" / name for name in
             ["control-known-answers.json", "conditions-known-answers.json"]]
    if args.record:
        if args.gnu_repository is None:
            parser.error("--gnu-repository is required for recording")
        sources = {}
        for name in ["eval.c", "data.c", "json.c", "bytecode.c"]:
            data = subprocess.check_output(["git", "-C", str(args.gnu_repository),
                                            "show", f"{REVISION}:src/{name}"])
            sources[name] = digest(data)
        metadata = dict(regime="GNU byte-compiled / VM2 compiled",
                        world="fresh GNU process and VM2 per case; no json.el preload",
                        gc="GNU automatic GC enabled; VM2 moving GC floor 1",
                        gnu_revision=REVISION,
                        gnu_binary_sha256=digest(Path(args.gnu).read_bytes()),
                        gnu_sources=sources, generator_sha256=digest(Path(__file__).read_bytes()),
                        source_sha256=source_digest(), compiler_id=compiler_id(args.binary),
                        binary_sha256=digest(Path(args.binary).read_bytes()))
        for path in paths:
            evidence = json.loads(path.read_text())
            for row in evidence["cases"]:
                row["expected"] = record(args.gnu, row["source"])
            evidence.update(metadata)
            evidence["command"] = "python3 tools/control_oracle.py --record --gnu-repository REPO --binary VM2"
            save(path, evidence)
        program = r'''(let (rows)
          (mapatoms (lambda (s) (when (get s 'error-conditions)
            (push (vector (symbol-name s)
                          (vconcat (mapcar #'symbol-name (get s 'error-conditions)))
                          (get s 'error-message)) rows))))
          (let* ((before (vconcat (mapcar #'symbol-name (get 'json-end-of-file 'error-conditions))))
                 (_ (require 'json))
                 (after (vconcat (mapcar #'symbol-name (get 'json-end-of-file 'error-conditions)))))
            (princ (decode-coding-string (json-serialize
              (vector emacs-version emacs-repository-version
                      (vconcat (sort rows (lambda (a b) (string< (aref a 0) (aref b 0)))))
                      (vector before after))) 'utf-8))))'''
        version, revision, rows, load_order = gnu_call(args.gnu, program)
        if (version, revision) != ("32.0.50", REVISION) or not rows:
            raise RuntimeError("empty or wrong catalog")
        save(ROOT / "evidence/conditions-catalog.json",
             dict(version=version, conditions=rows, json_load_order=load_order, **metadata))
    count = 0
    failures = []
    for path in paths:
        evidence = json.loads(path.read_text())
        if not evidence["cases"]:
            raise RuntimeError("empty cases")
        if evidence["source_sha256"] != source_digest():
            raise RuntimeError("candidate source differs from oracle identity")
        if evidence["compiler_id"] != compiler_id(args.binary):
            raise RuntimeError("candidate binary differs from oracle identity")
        for row in evidence["cases"]:
            with tempfile.TemporaryDirectory(prefix="control-candidate-") as directory:
                env = dict(os.environ, HOME=directory, TMPDIR=directory,
                           XDG_CONFIG_HOME=directory, XDG_CACHE_HOME=directory,
                           XDG_DATA_HOME=directory, XDG_STATE_HOME=directory)
                result = subprocess.run([args.binary, "--gc-floor", "1"],
                                        input=json.dumps(row["transport"]),
                                        text=True, capture_output=True, env=env, timeout=30)
            count += 1
            if result.returncode or result.stdout.strip() != row["expected"]:
                failures.append(dict(name=row["name"], expected=row["expected"],
                                     actual=result.stdout, stderr=result.stderr,
                                     exit=result.returncode))
    for failure in failures:
        print(json.dumps(failure))
    print(f"{count - len(failures)}/{count} fresh-world compiled control cases")
    raise SystemExit(bool(failures))


if __name__ == "__main__":
    main()
