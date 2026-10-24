#!/usr/bin/env python3
"""Mutation proof in a disposable VM2 copy; never edits the live candidate."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

crate = Path(__file__).resolve().parents[1]
repo = crate.parents[1]
source_files = ["command.rs", "keymap.rs", "lib.rs", "threaded.rs", "prims/callint.rs"]
report = {"source_sha256": {name: hashlib.sha256((crate / "src" / name).read_bytes()).hexdigest() for name in source_files}, "runs": []}
mutants = []
for command in ["SelfInsertCommand", "ForwardChar", "BackwardChar", "BeginningOfLine", "EndOfLine", "DeleteChar", "DeleteBackwardChar"]:
    mutants.append(("noop-" + command, "command.rs", "        use Primitive::*;", "        use Primitive::*;\n        if p == Primitive::" + command + " { return Ok(Value::NIL); }"))
mutants += [
    ("prefix-default-unbound", "lib.rs", 'self.slots[i].value = Value::NIL;\n            self.specials.insert(name.to_owned());', 'self.specials.insert(name.to_owned());'),
    ("prefix-late-special", "lib.rs", 'if name == "current-prefix-arg" {\n            self.slots[i].value = Value::NIL;\n            self.specials.insert(name.to_owned());\n        }', 'if name == "current-prefix-arg" { self.slots[i].value = Value::NIL; }'),
    ("interactive-numeric", "prims/callint.rs", "b'p' => { let n = prefix_numeric(cx, prefix); push(cx, &mut saved, n, n) }", "b'p' => { let n = fixnum(prefix_numeric(cx, prefix).integer_value().unwrap() + 1); push(cx, &mut saved, n, n) }"),
    ("unbound-no-bell", "command.rs", 'command:None,signal:None,bell:true', 'command:None,signal:None,bell:false'),
    ("boundary-wrong-condition", "command.rs", 'let condition=self.intern(s);', 'let condition=self.intern("error");'),
    ("nil-range-override", "keymap.rs", '(range_match && found.is_none())', 'range_match'),
    ("no-character-range", "keymap.rs", 'key == event || (range_match && found.is_none())', 'key == event'),
    ("retained-event-objects", "command.rs", '        self.ensure_keymaps();', '''        self.ensure_keymaps();
        let slot=self.slot("#:mutant-retained");
        let old=self.slots[slot].value;
        let old=if old==Value::UNBOUND {Value::NIL} else {old};
        let retained=self.keymap_cons(Value::TRUE,old);
        self.slots[slot].value=retained;'''),
    ("retained-container-bytes", "command.rs", '        self.ensure_keymaps();', '''        self.ensure_keymaps();
        let slot=self.slot("#:mutant-retained");
        let old=self.slots[slot].value;
        let bytes=format!("{}xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",self.source_string(old).unwrap_or(""));
        self.slots[slot].value=self.allocate_object(Object::String(Box::new(data::UnicodeString::new(&bytes))));'''),
]
with tempfile.TemporaryDirectory(prefix=".command-mutants-", dir=crate / "tools") as temp:
    root = Path(temp)
    candidate = root / "vm2"
    candidate.mkdir()
    shutil.copytree(crate / "src", candidate / "src")
    shutil.copy2(crate / "build.rs", candidate / "build.rs")
    (candidate / "tests").mkdir()
    shutil.copy2(crate / "tests" / "command.rs", candidate / "tests" / "command.rs")
    manifest = (crate / "Cargo.toml").read_text()
    for dependency in ["romacs-layout-input", "romacs-face", "romacs-regex-engine"]:
        manifest = manifest.replace('path = "../' + dependency + '"', 'path = "' + str(repo / "crates" / dependency) + '"')
    manifest = re.sub(r'\[dev-dependencies\]\n.*?(?=\n\[|\Z)', '', manifest, flags=re.S)
    (candidate / "Cargo.toml").write_text(manifest)
    (root / "Cargo.toml").write_text('[workspace]\nmembers=["vm2"]\nresolver="2"\n')
    shutil.copy2(repo / "Cargo.lock", root / "Cargo.lock")
    originals = {name: (candidate / "src" / name).read_text() for name in source_files}
    env = dict(os.environ, CARGO_TARGET_DIR=str(repo / "target" / "command-mutations"))
    # crate/build.rs expects ../../Cargo.lock, just as in the real workspace.
    (root / "crates").mkdir()
    candidate.rename(root / "crates" / "romacs-vm")
    candidate = root / "crates" / "romacs-vm"
    (root / "Cargo.toml").write_text('[workspace]\nmembers=["crates/romacs-vm"]\nresolver="2"\n')
    for name, file, old, new in [("baseline", None, None, None)] + mutants:
        for source, content in originals.items():
            (candidate / "src" / source).write_text(content)
        if file:
            content = originals[file]
            assert old is not None and new is not None
            pattern = r"\s*".join(re.escape(c) for c in old if not c.isspace())
            matches = list(re.finditer(pattern, content))
            if len(matches) != 1:
                raise RuntimeError(f"{name}: expected unique mutation site, got {len(matches)}")
            match = matches[0]
            (candidate / "src" / file).write_text(content[:match.start()] + new + content[match.end():])
        result = subprocess.run(["cargo", "test", "-p", "romacs-vm", "--test", "command", "--", "--nocapture"], cwd=root, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        failed = re.findall(r'^test (\w+) \.\.\. FAILED$', result.stdout, re.M)
        valid = result.returncode == 0 if name == "baseline" else result.returncode == 101 and bool(failed) and "panicked at" in result.stdout and "could not compile" not in result.stdout
        report["runs"].append({"name": name, "exit_code": result.returncode, "failed_tests": failed, "valid": valid, "output": result.stdout})
        print(name, "PASS" if valid else "INVALID", failed, flush=True)
        if not valid:
            print(result.stdout, flush=True)
            break
out = crate / "tools" / "command-mutations.json"
out.write_text(json.dumps(report, indent=2) + "\n")
assert len(report["runs"]) == len(mutants) + 1 and all(run["valid"] for run in report["runs"]), out
print(f"Verified {len(mutants)} assertion-killed mutants; report {out}")
