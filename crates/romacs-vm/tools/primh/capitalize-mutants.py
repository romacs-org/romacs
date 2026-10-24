#!/usr/bin/env python3
"""Run bounded compiling mutants and retain assertion-level evidence."""
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[4]
OUT = ROOT / "crates/romacs-vm/evidence/capitalize"
OUT.mkdir(parents=True, exist_ok=True)
PRIM = ROOT / "crates/romacs-vm/src/prims/capitalize.rs"
LEAF = ROOT / "crates/romacs-vm/src/prim.rs"
LIB = ROOT / "crates/romacs-vm/src/lib.rs"
VARIABLES = ROOT / "crates/romacs-vm/src/variable_runtime.rs"
MUTANTS = [
    ("no-int-narrowing", PRIM, "let ch = n as i32;", "let ch = if n > i64::from(i32::MAX) { i32::MAX } else { n as i32 };", "capitalize", "CAPITALIZE_NATIVE_CONTRACT"),
    ("no-title", PRIM, "|r| r.2)", "|_r| &fallback[..])", "capitalize", "CAPITALIZE_NATIVE_CONTRACT"),
    ("no-lower", PRIM, "|r| r.3)", "|_r| &fallback[..])", "capitalize", "CAPITALIZE_NATIVE_CONTRACT"),
    ("no-final-sigma", PRIM, "if inword && code == 0x3a3", "if false && inword && code == 0x3a3", "capitalize", "CAPITALIZE_NATIVE_CONTRACT"),
    ("wrong-default-word-syntax", PRIM, "WORD_RANGES[i].2", "{ let _ = i; 1 }", "capitalize", "CAPITALIZE_NATIVE_CONTRACT"),
    ("ignore-current-syntax", LEAF, "let class = lookup(table, code)", "let class = lookup(table, code).filter(|_| false)", "capitalize", "CAPITALIZE_NATIVE_CONTRACT"),
    ("ignore-local-case-symbols", LEAF, "let value = self.vm.variable_value(slot).map_err(Fault::Error)?;\n                value != Value::NIL && value != Value::UNBOUND", "let value = self.vm.slots[slot].value;\n                value != Value::NIL && value != Value::UNBOUND", "capitalize", "CAPITALIZE_NATIVE_CONTRACT"),
    ("no-character-casing", PRIM, "mapping(code).map_or(code, |row| row.1)", "mapping(code).map_or(code, |_row| code)", "capitalize", "CAPITALIZE_NATIVE_CONTRACT"),
    ("drop-unibyte-properties", PRIM, "cx.buffer_name_string(object, bytes, false)", "cx.make_string(bytes, false)", "capitalize", "CAPITALIZE_NATIVE_CONTRACT"),
    ("not-automatically-local", VARIABLES, 'slot == self.truncate_lines_slot || self.names.get("case-symbols-as-words") == Some(&slot)', 'slot == self.truncate_lines_slot', "capitalize", "CAPITALIZE_NATIVE_CONTRACT"),
    ("replay-no-lower", PRIM, "|r| r.3)", "|_r| &fallback[..])", "primh_capitalize", "NEW DIVERGENCE"),
    ("frontier-missing-capitalize", PRIM, 'name = "capitalize"', 'name = "capitalize-mutant"', "custom_bootstrap", "CUSTOM_FILES_FRONTIER_MOVED"),
]
results = []
for name, path, old, new, target, assertion in MUTANTS:
    original = path.read_bytes()
    source = original.decode()
    assert source.count(old) == 1, (name, source.count(old))
    try:
        path.write_text(source.replace(old, new))
        command = ["cargo", "test", "-p", "romacs-vm", "--test", target, "--", "--nocapture"]
        run = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300)
        (OUT / f"mutant-{name}.log").write_bytes(run.stdout)
        text = run.stdout.decode(errors="replace")
        killed = run.returncode != 0 and "could not compile" not in text and assertion in text and "panicked at" in text
        row = dict(name=name, command=command, exit=run.returncode, assertion=assertion, killed_at_assertion=killed,
                   source_sha256=hashlib.sha256(original).hexdigest())
        results.append(row)
        print(json.dumps(row), flush=True)
    finally:
        path.write_bytes(original)
        assert hashlib.sha256(path.read_bytes()).digest() == hashlib.sha256(original).digest()
    (OUT / "mutants.json").write_text(json.dumps(results, indent=2) + "\n")
assert all(row["killed_at_assertion"] for row in results), "unresolved mutant"
