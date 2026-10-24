#!/usr/bin/env python3
"""Replay the checked-in annotation answers against live GNU, with private HOME."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def lisp(form):
    if isinstance(form, dict) and set(form) == {"string"}:
        return json.dumps(form["string"], ensure_ascii=False)
    if isinstance(form, list):
        return "(" + " ".join(map(lisp, form)) + ")"
    if isinstance(form, (str, int)):
        return str(form)
    raise ValueError(f"unsupported form transport: {form!r}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle", default=os.environ.get("ROMACS_FORCE_ORACLE_PATH", "emacs"))
    args = parser.parse_args()
    fixture_dir = Path(__file__).resolve().parents[1] / "tests/fixtures"
    rows = [json.loads(line) for name in ("annotation_m1.jsonl", "annotation_presence.jsonl")
            for line in (fixture_dir / name).read_text().splitlines()]
    with tempfile.TemporaryDirectory(prefix="annotation-m1-") as directory:
        home = Path(directory)
        script = home / "oracle.el"
        env = dict(os.environ, HOME=str(home), XDG_CONFIG_HOME=str(home / "config"),
                   XDG_DATA_HOME=str(home / "data"), XDG_CACHE_HOME=str(home / "cache"),
                   XDG_STATE_HOME=str(home / "state"), XDG_RUNTIME_DIR=str(home / "run"))
        answers = []
        # Match the VM's fresh-world-per-case contract: function redefinitions
        # and symbol plists from one case must not contaminate the next.
        for index, row in enumerate(rows):
            script.write_text(";; -*- lexical-binding: t; -*-\n(prin1 " + lisp(row["form"]) + ") (terpri)\n")
            result = subprocess.run([args.oracle, "-Q", "--batch", "-l", str(script)],
                                    env=env, text=True, capture_output=True, check=False, timeout=120)
            if result.returncode:
                raise RuntimeError(f"GNU case {index} exited {result.returncode}\n{result.stdout}\n{result.stderr}")
            lines = result.stdout.splitlines()
            if len(lines) != 1:
                raise AssertionError(f"GNU case {index}: expected one record, received {len(lines)}")
            answers.append(lines[0])
    if len(answers) != len(rows):
        raise AssertionError(f"expected {len(rows)} records, received {len(answers)}")
    failures = [{"case": index, "expected": row["answer"], "actual": answer}
                for index, (row, answer) in enumerate(zip(rows, answers)) if row["answer"] != answer]
    print(json.dumps({"cases": len(rows), "failures": failures}, ensure_ascii=False))
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
