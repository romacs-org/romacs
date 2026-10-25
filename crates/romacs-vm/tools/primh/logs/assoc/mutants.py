#!/usr/bin/env python3
"""Source mutants of the assoc/delete/nlistp bodies; each must fail the primh
replay assertion of tests/primh_fns_equal.rs. Restores the source afterwards.

Run from an owned checkout with the prepared project environment:
  python3 crates/romacs-vm/tools/primh/logs/assoc/mutants.py
"""
import re, subprocess, sys
from pathlib import Path

ROOT = Path.cwd()
SRC = ROOT / "crates/romacs-vm/src/prims/fns_equal.rs"
LOGS = ROOT / "crates/romacs-vm/tools/primh/logs"
MUTANTS = [
    # (primitive, name, old, new)
    ("assoc", "eq-not-equal",
     "if candidate == key || equal(cx, candidate, key)? {", "if candidate == key {"),
    ("assoc", "ignores-testfn",
     "if testfn != Value::NIL {", "if false && testfn != Value::NIL {"),
    ("assoc", "returns-testfn-value",
     "if matched != Value::NIL {\n        return Ok(Step::Done(element));",
     "if matched != Value::NIL {\n        return Ok(Step::Done(matched));"),
    ("assoc", "list-end-names-tail",
     "if tail != Value::NIL { return Err(Fault::Wrong(\"listp\", alist)); }",
     "if tail != Value::NIL { return Err(Fault::Wrong(\"listp\", tail)); }"),
    ("delete", "nondestructive",
     "else { cx.setcdr(previous, cdr)?; }", "else { let _ = cdr; }"),
    ("delete", "wrong-head",
     "if previous == Value::NIL { seq = cdr; }", "if previous == Value::NIL { }"),
    ("delete", "vector-always-new",
     "return Ok(if kept.len() == items.len() { seq } else { cx.vector(&kept) });",
     "return Ok(cx.vector(&kept));"),
    ("delete", "string-bytes-not-chars",
     "let (c, width) = if string.is_multibyte() {", "let (c, width) = if false && string.is_multibyte() {"),
    ("delete", "eq-not-equal",
     "if equal(cx, elt, car)? {", "if elt == car {"),
    ("nlistp", "inverted",
     "Ok(if object == Value::NIL || cx.pair(object).is_some() { Value::NIL } else { Value::TRUE })",
     "Ok(if object == Value::NIL || cx.pair(object).is_some() { Value::TRUE } else { Value::NIL })"),
]


def run(log):
    with open(log, "w") as out:
        p = subprocess.run(["cargo", "test", "-p", "romacs-vm", "--test", "primh_fns_equal"],
                           stdout=out, stderr=subprocess.STDOUT)
    return p.returncode, log.read_text()


def main():
    original = SRC.read_text()
    summary = []
    try:
        for prim, name, old, new in MUTANTS:
            assert original.count(old) == 1, (name, old)
            SRC.write_text(original.replace(old, new))
            log = LOGS / prim / f"mutant-{name}.log"
            rc, text = run(log)
            failed = sorted(set(re.findall(r"test (primh_\w+) \.\.\. FAILED", text)))
            at_replay = len(re.findall(r"panicked at crates/romacs-vm/tests/primh_fns_equal\.rs:79:5", text))
            counts = re.findall(r'^(\d+) "([a-z-]+)":', text, re.M)
            compiled = "error[E" not in text and "could not compile" not in text
            killed = rc != 0 and compiled and at_replay == len(failed) and f"primh_{prim}" in failed
            line = f"{prim}/{name}: rc={rc} compiled={compiled} failed={failed} replay-assertions={at_replay} divergences={counts} {'KILLED' if killed else 'SURVIVED'}"
            print(line, flush=True)
            summary.append((killed, line))
    finally:
        SRC.write_text(original)
    rc, text = run(LOGS / "assoc" / "mutants-restored.log")
    print(f"restored rc={rc}")
    (LOGS / "assoc" / "mutants.out").write_text("\n".join(l for _, l in summary) + f"\nrestored rc={rc}\n")
    return 0 if all(k for k, _ in summary) and rc == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
