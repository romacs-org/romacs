#!/usr/bin/env python3
"""Mutation proof for crates/romacs-vm/tests/expansion_objects.rs.

Each mutant edits one protected line of the subject, runs the test binary,
records which tests fail, and restores the file byte-for-byte. A mutant
that leaves every test green is reported as SURVIVED (a gap). Run from the
repository root with the lane environment (cargo on PATH)."""
import subprocess, sys, pathlib, re, json

ROOT = pathlib.Path(__file__).resolve().parents[4]
VM = ROOT / "crates/romacs-vm/src"
MUTANTS = [
    ("M1 expansion->strict in expand_macro", VM / "source_cache.rs",
     "            self.expansion_form(value, 0)\n        })();", "            self.source_form(value, 0)\n        })();"),
    ("M2 eval datum strict", VM / "eval_form.rs",
     "let mut form = self.expansion_form(datum, 0)?;", "let mut form = self.source_form(datum, 0)?;"),
    ("M3 no function-object head call", VM / "lib.rs",
     "Form::Object(v) if matches!(v.tag(), 4 | 6) => {\n                        let mut call",
     "Form::Object(v) if false && matches!(v.tag(), 4 | 6) => {\n                        let mut call"),
    ("M4 quote copies the object away", VM / "data.rs",
     "Form::Object(v) => Ok(*v),", "Form::Object(_) => Ok(Value::NIL),"),
    ("M5 tag-0 objects not carried", VM / "reader.rs",
     "_ if objects => Ok(Form::Object(value)),", "_ if false => Ok(Form::Object(value)),"),
    ("M6 invalid-function data dropped", VM / "eval_form.rs",
     'Some(("invalid-function", vec![head]))', 'Some(("invalid-function", vec![]))'),
    ("M7 macro expansion may collect", VM / "source_cache.rs",
     "        let threshold = self.gc_threshold;\n        self.gc_threshold = usize::MAX;\n        let result = (|| {",
     "        let threshold = self.gc_threshold;\n        let result = (|| {"),
    ("M9 dotted call on a function object is invalid-function", VM / "lib.rs",
     "|| matches!(&items[0], Form::Object(v) if matches!(v.tag(), 4 | 6));",
     "|| matches!(&items[0], Form::Object(v) if false && matches!(v.tag(), 4 | 6));"),
    ("M8 lazy body strict", VM / "lib.rs",
     "let body = match self.expansion_form(lazy.body, 0)? {", "let body = match self.source_form(lazy.body, 0)? {"),
]

def run():
    p = subprocess.run(["cargo", "test", "-p", "romacs-vm", "--test", "expansion_objects"],
                       cwd=ROOT, capture_output=True, text=True)
    out = p.stdout + p.stderr
    failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M)))
    compiled = "test result:" in out
    return p.returncode, failed, compiled, out

results = []
for name, path, old, new in MUTANTS:
    original = path.read_bytes()
    text = original.decode()
    if text.count(old) != 1:
        print(f"{name}: anchor not unique/found ({text.count(old)})", file=sys.stderr)
        sys.exit(2)
    path.write_text(text.replace(old, new))
    try:
        code, failed, compiled, out = run()
    finally:
        path.write_bytes(original)
    verdict = "KILLED" if failed else ("NO-BUILD" if not compiled else "SURVIVED")
    results.append({"mutant": name, "verdict": verdict, "exit": code, "failed_tests": failed})
    print(json.dumps(results[-1]), flush=True)
code, failed, compiled, _ = run()
print(json.dumps({"restored": "green" if code == 0 else "RED", "failed_tests": failed}))
sys.exit(0 if all(r["verdict"] == "KILLED" for r in results) and code == 0 else 1)
