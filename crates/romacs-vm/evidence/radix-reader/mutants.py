#!/usr/bin/env python3
"""Named-assertion mutation checks; never treat build failures/timeouts as kills.
Run with --logs OUTSIDE the worktree. Restores the exact source in finally.
"""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument("--logs", required=True, type=pathlib.Path)
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parents[4]
logs = args.logs.resolve()
if logs.is_relative_to(root):
    parser.error("logs must be outside worktree")
logs.mkdir(parents=True, exist_ok=True)
reader = root / "crates/romacs-vm/src/reader.rs"
original = reader.read_text()
mutations = [
    ("octal-base", "'o' | 'O' => self.radix_integer(8)", "'o' | 'O' => self.radix_integer(16)", "radix_reader_matches_pinned_gnu", "RADIX_GNU_MISMATCH"),
    ("uppercase-prefix", "'x' | 'X' =>", "'x' =>", "radix_reader_matches_pinned_gnu", "RADIX_GNU_MISMATCH"),
    ("bad-digit", "valid &= c.to_digit(radix).is_some();", "valid = true;", "radix_reader_matches_pinned_gnu", "RADIX_GNU_MISMATCH"),
    ("missing-digit", "if self.at == digits || !valid", "if !valid", "radix_reader_matches_pinned_gnu", "RADIX_GNU_MISMATCH"),
    ("sign", "self.source[start..self.at], radix", "self.source[digits..self.at], radix", "radix_reader_matches_pinned_gnu", "RADIX_GNU_MISMATCH"),
    ("radix-bound", "!(2..=36).contains(&radix)", "!(3..=36).contains(&radix)", "radix_reader_matches_pinned_gnu", "RADIX_GNU_MISMATCH"),
    ("prefix-overflow", "self.syntax(&self.source[start..self.at].to_owned())", "self.syntax(\"overflow\")", "radix_reader_matches_pinned_gnu", "RADIX_GNU_MISMATCH"),
    ("signal-datum", 'if self.at == digits || !valid {\n            return Err(self.syntax(&format!("integer, radix {radix}")));', 'if self.at == digits || !valid {\n            return Err(self.syntax(&format!("bad radix {radix}")));', "radix_reader_matches_pinned_gnu", "RADIX_GNU_MISMATCH"),
    ("bignum-source-constant", "Object::Bignum(_) => Ok(Form::Object(crate::handle::FormObject::new(self, value)))", "Object::Bignum(_) => Ok(Form::Int(0))", "radix_bignum_constant_is_traced_through_compaction", "RADIX_CONSTANT_FORWARDING"),
]
mutations = [(reader, *m) for m in mutations]
cache = root / "crates/romacs-vm/src/source_cache.rs"
mutations.append((cache, "disk-form-object", "!contains_local_object(form)", "true", "radix_bignum_discarded_constants_with_disk_store_match_gnu", "RADIX_DISK_STORE_MISMATCH"))
originals = {path: path.read_text() for path, *_ in mutations}
def git(*args):
    return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()
identity = dict(commit=git("rev-parse", "HEAD"), tree=git("rev-parse", "HEAD^{tree}"),
                status=git("status", "--porcelain"),
                sources={str(p.relative_to(root)): hashlib.sha256(s.encode()).hexdigest() for p, s in originals.items()})
(logs / "identity.json").write_text(json.dumps(identity, indent=2) + "\n")
command = ["cargo", "test", "-p", "romacs-vm", "--test", "radix_reader", "--jobs", "2", "--"]
results = []
try:
    baseline = subprocess.run(command + ["--nocapture"], capture_output=True, text=True, timeout=180)
    (logs / "baseline.log").write_text(baseline.stdout + baseline.stderr)
    if baseline.returncode or "3 passed; 0 failed" not in baseline.stdout:
        raise SystemExit("baseline did not execute and pass all three tests")
    for path, name, old, new, test, message in mutations:
        original = originals[path]
        if original.count(old) != 1:
            raise SystemExit(f"mutation anchor must occur exactly once: {name}")
        path.write_text(original.replace(old, new))
        try:
            run = subprocess.run(command + [test, "--exact", "--nocapture"], capture_output=True, text=True, timeout=180)
        except subprocess.TimeoutExpired:
            raise SystemExit(f"TIMEOUT, not a kill: {name}")
        text = run.stdout + run.stderr
        (logs / f"{name}.log").write_text(text)
        killed = (run.returncode != 0 and f"thread '{test}'" in text and re.search(r"(?m)^(?:assertion .* failed: )?" + re.escape(message) + r"(?: .*)?$", text) is not None and "test result: FAILED. 0 passed; 1 failed" in text)
        results.append(dict(mutant=name, killed=killed, assertion=message, exit=run.returncode))
        print(json.dumps(results[-1]), flush=True)
        if not killed:
            raise SystemExit(f"NO TARGET ASSERTION KILL: {name}")
        path.write_text(original)
finally:
    for path, original in originals.items():
        path.write_text(original)
    (logs / "results.json").write_text(json.dumps(results, indent=2) + "\n")
