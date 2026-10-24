#!/usr/bin/env python3
"""Mutation proof for VM2-AUTOLOAD1: tests/autoload_load.rs and the unit
tests in src/autoload.rs. Each mutant edits one protected line, runs both
test sets, records which tests fail, and restores the file byte-for-byte.
SURVIVED means a gap. Run from the repository root with the lane env."""
import subprocess, sys, pathlib, re, json

ROOT = pathlib.Path(__file__).resolve().parents[4]
VM = ROOT / "crates/romacs-vm/src"
A = VM / "autoload.rs"
MUTANTS = [
    ("M1 call path does not load (invoke_full)", VM / "threaded.rs",
     "        if target.tag() == 3 && e.vm.autoload_object(target) {\n            return match autoload_then(e, r, function, target) {\n                Some(r) => t2_invoke_full",
     "        if false && target.tag() == 3 && e.vm.autoload_object(target) {\n            return match autoload_then(e, r, function, target) {\n                Some(r) => t2_invoke_full"),
    ("M2 direct call site passes the cell, not the symbol", VM / "threaded.rs",
     "if target.tag() == 7 || target.tag() == 3 && e.vm.autoload_object(target) {",
     "if target.tag() == 7 {"),
    ("M3 T2 call request does not load", VM / "threaded.rs",
     "            return match autoload_then(e, r, function, target) {\n                Some(r) => t2_dispatch(",
     "            return match None::<*mut Value> {\n                Some(r) => t2_dispatch("),
    ("M4 macro autoload not loaded before expansion", VM / "source_cache.rs",
     "if !self.macros.contains(&slot) && self.autoload_macro(self.slots[slot].function) {",
     "if false && !self.macros.contains(&slot) && self.autoload_macro(self.slots[slot].function) {"),
    ("M5 no undo on failure", A,
     "        if result.is_err() {\n            self.un_autoload(queue);\n        }",
     "        if result.is_err() {\n        }"),
    ("M6 provide records nothing", VM / "loader.rs",
     "                self.autoload_note_provide();\n", ""),
    ("M7 defalias records nothing", VM / "bootstrap.rs",
     "                self.autoload_note_defalias(slot);\n", ""),
    ("M8 native defun records nothing", VM / "threaded.rs",
     "                    e.vm.autoload_note_defalias(slot);\n", ""),
    ("M9 match data not restored", A,
     "        let restored = self.regexp_set_match_data(saved, Value::TRUE);",
     "        let restored: std::result::Result<Value, ()> = { let _ = saved; Ok(Value::NIL) };"),
    ("M10 no failed-to-define check", A,
     "        if self.equal(fun, fundef).map_err(|_| \"equal failed comparing an autoload\")? {",
     "        if false && self.equal(fun, fundef).map_err(|_| \"equal failed comparing an autoload\")? {"),
    ("M11 MACRO-ONLY ignored", A,
     "        if self.symbol_named(macro_only, \"macro\") && !is_macro {",
     "        if false && self.symbol_named(macro_only, \"macro\") && !is_macro {"),
    ("M12 FUNNAME not checked", A,
     "        if !(funname == Value::NIL || funname == Value::TRUE || (funname.tag() == 7 && funname != Value::UNBOUND)) {",
     "        if false {"),
    ("M13 FUNDEF read from an untraced local after the load", A,
     "        let fundef = self.slots[pending].value;\n", ""),
    ("M14 queue left active after the load (retained records)", A,
     "        self.dynamic.push((queue, self.slots[queue].value, buffer, false));\n", ""),
    ("M15 require bypasses the queue", VM / "loader.rs",
     "                let path = self.load_with_autoload_queue(file, noerror, must_suffix)?;",
     "                let path = { let name = self.source_string(file)?.to_owned(); match self.source_path(&name, false)? { Some(p) => { self.load_file(&p)?; p } None if noerror => std::path::PathBuf::new(), None => return Err(format!(\"file missing: {name}\")) } };"),
    ("M16 missing file is not file-missing", A,
     "            return Err(self.signal_error(\"file-missing\", &[text, reason, file], format!(\"file missing: {name}\")));",
     "            return Err(self.signal_error(\"error\", &[text, reason, file], format!(\"file missing: {name}\")));"),
    ("M17 macro aliases not followed", VM / "source_cache.rs",
     "        let slot = self.macro_alias_end(head);\n        if !self.macros.contains(&slot) && self.autoload_macro",
     "        let slot = head;\n        if !self.macros.contains(&slot) && self.autoload_macro"),
    ("M18 stale load escape kept (neither consumer clears it)", [
        (VM / "source_cache.rs", "                return Err(match self.propagated_escape.take() {", "                return Err(match None::<threaded::Escape> {"),
        (VM / "threaded.rs", "        e.vm.propagated_escape = None;\n        let result = e.vm.loader_primitive(p, &args);", "        let result = e.vm.loader_primitive(p, &args);"),
    ], None, None),
    ("M19 alias chain not re-resolved after the load", VM / "source_cache.rs",
     "        let slot = self.macro_alias_end(head);\n        if !self.macros.contains(&slot) {\n            return Ok(None);",
     "        if !self.macros.contains(&slot) {\n            return Ok(None);"),
    ("M20 cache validity ignores the alias target", VM / "source_cache.rs",
     "            && e.calls.iter().all(|(s, v)| self.call_is_macro(*s) == *v)",
     "            && e.calls.iter().all(|(s, v)| (self.macros.contains(s) || self.autoload_macro(self.slots[*s].function)) == *v)"),
    ("M21 cache validity ignores macro autoloads", VM / "source_cache.rs",
     "        self.macros.contains(&end) || self.autoload_macro(self.slots[end].function)\n    }",
     "        self.macros.contains(&end)\n    }"),
]

def run():
    out, codes = "", []
    for args in (["--test", "autoload_load"], ["--lib", "autoload::"]):
        p = subprocess.run(["cargo", "test", "-p", "romacs-vm", *args], cwd=ROOT, capture_output=True, text=True)
        out += p.stdout + p.stderr
        codes.append(p.returncode)
    failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M)))
    compiled = out.count("test result:") == 2
    return max(codes), failed, compiled, out

results = []
for name, path, old, new in MUTANTS:
    edits = path if isinstance(path, list) else [(path, old, new)]
    originals = {}
    for p, o, n in edits:
        originals.setdefault(p, p.read_bytes())
        text = p.read_text()
        if text.count(o) != 1:
            for q, b in originals.items():
                q.write_bytes(b)
            print(f"{name}: anchor not unique/found ({text.count(o)}) in {p.name}", file=sys.stderr)
            sys.exit(2)
        p.write_text(text.replace(o, n))
    try:
        code, failed, compiled, out = run()
    finally:
        for q, b in originals.items():
            q.write_bytes(b)
    verdict = "KILLED" if failed else ("NO-BUILD" if not compiled else "SURVIVED")
    results.append({"mutant": name, "verdict": verdict, "exit": code, "failed_tests": failed})
    if verdict == "NO-BUILD":
        results[-1]["build_error"] = "\n".join(l for l in out.splitlines() if l.startswith("error"))[:600]
    print(json.dumps(results[-1]), flush=True)
code, failed, compiled, _ = run()
print(json.dumps({"restored": "green" if code == 0 else "RED", "failed_tests": failed}))
sys.exit(0 if all(r["verdict"] == "KILLED" for r in results) and code == 0 else 1)
