#!/usr/bin/env python3
"""Source mutants of the keywordp body; each must fail the primh replay
assertion of tests/primh_data_kw.rs. Restores the source afterwards.

Run from an owned checkout with the prepared project environment:
  python3 crates/romacs-vm/tools/primh/logs/keywordp/mutants.py
"""
import re, subprocess, sys
from pathlib import Path

ROOT = Path.cwd()
SRC = ROOT / "crates/romacs-vm/src/prims/data_kw.rs"
LOGS = ROOT / "crates/romacs-vm/tools/primh/logs/keywordp"
TEST = "primh_data_kw"
MUTANTS = [
    # (name, old, new)
    ("ignores-colon",
     "name.as_bytes().first() == Some(&b':') && cx.obarray_lookup",
     "cx.obarray_lookup"),
    ("ignores-interned",
     "name.as_bytes().first() == Some(&b':') && cx.obarray_lookup(&name) == Some(object)",
     "name.as_bytes().first() == Some(&b':')"),
    ("checks-last-char",
     "name.as_bytes().first() == Some(&b':')",
     "name.as_bytes().last() == Some(&b':')"),
    ("returns-symbol",
     "Ok(if is_keyword(cx, object) { Value::TRUE } else { Value::NIL })",
     "Ok(if is_keyword(cx, object) { object } else { Value::NIL })"),
    ("inverted",
     "Ok(if is_keyword(cx, object) { Value::TRUE } else { Value::NIL })",
     "Ok(if is_keyword(cx, object) { Value::NIL } else { Value::TRUE })"),
]


def run(log):
    with open(log, "w") as out:
        p = subprocess.run(["cargo", "test", "-p", "romacs-vm", "--test", TEST],
                           stdout=out, stderr=subprocess.STDOUT)
    return p.returncode, log.read_text()


def main():
    original = SRC.read_text()
    summary = []
    try:
        for name, old, new in MUTANTS:
            assert original.count(old) == 1, (name, old)
            SRC.write_text(original.replace(old, new))
            log = LOGS / f"mutant-{name}.log"
            rc, text = run(log)
            failed = sorted(set(re.findall(r"test (primh_\w+) \.\.\. FAILED", text)))
            at_replay = len(re.findall(rf"panicked at crates/romacs-vm/tests/{TEST}\.rs:79:5", text))
            counts = re.findall(r'^(\d+) "([a-z-]+)":', text, re.M)
            compiled = "error[E" not in text and "could not compile" not in text
            killed = rc != 0 and compiled and at_replay == len(failed) and failed == ["primh_keywordp"]
            line = f"keywordp/{name}: rc={rc} compiled={compiled} failed={failed} replay-assertions={at_replay} divergences={counts} {'KILLED' if killed else 'SURVIVED'}"
            print(line, flush=True)
            summary.append((killed, line))
    finally:
        SRC.write_text(original)
    rc, text = run(LOGS / "mutants-restored.log")
    print(f"restored rc={rc}")
    (LOGS / "mutants.out").write_text("\n".join(l for _, l in summary) + f"\nrestored rc={rc}\n")
    return 0 if all(k for k, _ in summary) and rc == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
