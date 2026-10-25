#!/usr/bin/env python3
"""Mutants for the heap boundary (vm2/heapbound).

Each mutant edits one source file, asserts the edit applied (a mutant that
fails to apply is indistinguishable from a survivor), runs its target test and
requires failure AT the expected assertion text. The file is restored from its
original bytes in a finally block and the restore is verified.

Kinds:
  runtime  - cargo test failure must contain EXPECT in the panic output.
  compile  - trybuild must report EXPECT for the named case (the property is
             that the program stops being rejected), or the library itself
             must fail to compile with EXPECT.
Run from an owned checkout with the prepared project environment:
  python3 crates/romacs-vm/evidence/heapbound/mutants.py [LABEL...]
"""
import json, pathlib, re, subprocess, sys, time

ROOT = pathlib.Path(__file__).resolve().parents[4]
VM2 = "crates/romacs-vm/"

MUTANTS = [
    # label, kind, file, old, new, test selector, expected text
    ("rooted-no-write-back", "runtime", VM2 + "src/handle.rs",
     "            slot.store(value.0, Ordering::Relaxed);\n",
     "            let _ = (slot, value);\n",
     ["--test", "heap_boundary", "rooted_handle_resolves_after_compaction_and_releases_on_drop"],
     "rooted handle must resolve to the same value after compaction"),
    ("rooted-not-traced", "runtime", VM2 + "src/handle.rs",
     "        if live.is_empty() {\n            return self.collect_marked(roots);",
     "        if true {\n            return self.collect_marked(roots);",
     ["--test", "heap_boundary", "rooted_handle_resolves_after_compaction_and_releases_on_drop"],
     "rooted handle must resolve to the same value after compaction"),
    ("rooted-drop-leaks", "runtime", VM2 + "src/handle.rs",
     "        slots.push(Arc::downgrade(&slot));\n",
     "        slots.push(Arc::downgrade(&slot));\n        std::mem::forget(slot.clone());\n",
     ["--test", "heap_boundary", "rooted_handle_resolves_after_compaction_and_releases_on_drop"],
     "dropping the handle must release it: live_objects/live_bytes delta"),
    ("rooted-any-vm", "runtime", VM2 + "src/handle.rs",
     "        if vm.rooted.id != self.vm {",
     "        if false && vm.rooted.id != self.vm {",
     ["--test", "heap_boundary", "rooted_handle_is_bound_to_its_vm"],
     "a handle must not resolve in another VM"),
    ("rooted-unbounded-slots", "runtime", VM2 + "src/handle.rs",
     "        if slots.len() >= 64 && slots.len().is_power_of_two() {",
     "        if false && slots.len() >= 64 && slots.len().is_power_of_two() {",
     ["--lib", "handle::tests::dropped_rooted_slots_are_bounded_without_collection"],
     "dropped Rooted slots retained without collection"),
    ("poll-never-collects", "runtime", VM2 + "src/threaded.rs",
     "    if e.vm.heap_objects() >= e.vm.gc_threshold {\n        // Snapshot every active Value copy",
     "    if false && e.vm.heap_objects() >= e.vm.gc_threshold {\n        // Snapshot every active Value copy",
     ["--test", "heap_boundary", "collection_runs_inside_one_request_not_only_at_its_boundary"],
     "mid-request collection"),
    ("local-unbranded", "compile", VM2 + "src/handle.rs",
     "    pub fn eval_source(&mut self, source: &str) -> Result<Local<'_>> {\n        let value = self.eval_source_raw(source)?;\n        Ok(Local::new(self, value))",
     "    pub fn eval_source(&mut self, source: &str) -> Result<Local<'static>> {\n        let value = self.eval_source_raw(source)?;\n        Ok(Local::new(unsafe { &*(self as *const Vm) }, value))",
     ["--test", "heap_boundary", "heap_boundary_is_enforced_by_the_compiler"],
     "local_across_eval.rs [should fail to compile] ... error\nExpected test case to fail to compile, but it succeeded."),
    ("value-public", "compile", VM2 + "src/lib.rs",
     "pub(crate) struct Value(u64);",
     "pub struct Value(u64);",
     ["--test", "heap_boundary", "heap_boundary_is_enforced_by_the_compiler"],
     "raw_value_is_unnameable.rs [should fail to compile] ... error\nExpected test case to fail to compile, but it succeeded."),
    ("collect-public", "compile", VM2 + "src/handle.rs",
     "    pub(crate) fn collect(&mut self, roots: &mut [Value]) {",
     "    pub fn collect(&mut self, roots: &mut [Value]) {",
     ["--test", "heap_boundary", "heap_boundary_is_enforced_by_the_compiler"],
     "type `Value` is more private than the item `handle::<impl Vm>::collect`"),
]


def run(selector):
    cmd = ["cargo", "test", "-p", "romacs-vm", "-j", "3", *selector, "--", "--exact"]
    if selector[0] == "--lib":
        cmd = cmd[:-1]
    t = time.time()
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    # trybuild colours its verdicts even into a pipe; match on plain text.
    out = re.sub(r"\x1b\[[0-9;]*m", "", p.stdout + p.stderr)
    return p.returncode, out, round(time.time() - t)


def main():
    wanted = set(sys.argv[1:])
    unknown = wanted - {m[0] for m in MUTANTS}
    if unknown:
        sys.exit(f"unknown mutant labels: {sorted(unknown)}")
    outcomes = []
    for label, kind, file, old, new, selector, expect in MUTANTS:
        if wanted and label not in wanted:
            continue
        path = ROOT / file
        original = path.read_bytes()
        text = original.decode()
        assert text.count(old) == 1, f"{label}: anchor occurs {text.count(old)} times"
        try:
            path.write_text(text.replace(old, new))
            applied = path.read_bytes() != original
            assert applied, f"{label}: mutant did not change {file}"
            code, out, secs = run(selector)
        finally:
            path.write_bytes(original)
        assert path.read_bytes() == original, f"{label}: restore failed"
        killed = code != 0 and expect in out
        tail = [l for l in out.splitlines() if expect.splitlines()[0] in l][:2]
        outcomes.append(dict(mutant=label, kind=kind, file=file, applied=applied,
                             exit=code, seconds=secs, expected=expect,
                             verdict="KILLED-AT-ASSERTION" if killed else
                             ("FAILED-NOT-AT-ASSERTION" if code else "SURVIVED"),
                             evidence=tail))
        print(json.dumps(outcomes[-1]), flush=True)
        if not killed:
            (ROOT / f"target-mutant-{label}.log").write_text(out)
    if not outcomes:
        sys.exit("no mutant ran")
    return 0 if all(o["verdict"] == "KILLED-AT-ASSERTION" for o in outcomes) else 1


if __name__ == "__main__":
    sys.exit(main())
