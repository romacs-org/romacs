#!/usr/bin/env python3
"""Re-runnable S109 mutants. Restore every source even when a run fails.

Run sequentially in the private candidate worktree with no other writers:
  python3 crates/romacs-vm/tools/fixnum_mutants.py --results SCRATCH_DIR
Each mutant must compile, run its named test, and panic at the expected assertion.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[3]
CRATE = ROOT / "crates/romacs-vm"
UNIT = "immediate_hashes_and_position_hashes_remain_distinct_after_compaction"
MUTANTS = [
    ("old-range", "src/lib.rs", "Self::MIN_FIXNUM..=Self::MAX_FIXNUM", "-(1i64 << 60)..(1i64 << 60)", "fixnums", "widened_fixnums_are_injective_and_roundtrip", "S109 all GNU fixnums must be immediate"),
    ("old-encode", "src/lib.rs", "((n as u64) << 2) | 1", "((n as u64) << 3) | 1", "fixnums", "widened_fixnums_are_injective_and_roundtrip", "S109 decode boundary"),
    ("old-decode", "src/lib.rs", "then_some((self.0 as i64) >> 2)", "then_some((self.0 as i64) >> 3)", "fixnums", "widened_fixnums_are_injective_and_roundtrip", "S109 decode boundary"),
    ("old-inc-step", "src/threaded.rs", ".checked_add(4)", ".checked_add(8)", "fixnums", "widened_fast_arithmetic_and_identity", "S109 fast arithmetic"),
    ("false-integer-predicate", "src/surface.rs", "Primitive::Integerp | Primitive::Fixnump => value.integer_value().is_some()", "Primitive::Integerp | Primitive::Fixnump => false", "fixnums", "fixnum_constants_and_predicates", "S109 integer predicates"),
    ("mutable-bounds", "src/threaded.rs", "|| e.vm.fixnum_slots.contains(&slot)", "|| false", "fixnums", "fixnum_constants_and_predicates", "S109 fixnum bounds are constants"),
    ("weak-immediates-die", "src/gc.rs", "fn survives(marks: &[Vec<bool>; 4], v: Value) -> bool {", "fn survives(marks: &[Vec<bool>; 4], v: Value) -> bool {\n        if v.integer_value().is_some_and(|n| n >= 1 << 60 || n < -(1 << 60)) { return false; }", "fixnums", "widened_weak_keys_survive_real_compaction", "S109 weak immediate retention and lookup after move"),
    ("cell-read-dropped", "src/threaded.rs", "*r.add((*pc).a) = e.vm.cells[v.payload() as usize];", "*r.add((*pc).a) = Value::integer(0).unwrap();", "fixnums", "widened_captured_cells_and_literal_roots_survive_collection", "S109 relocated cell and wide immediate"),
    ("image-constant-truncated", "src/code_image.rs", "Const(a, v) => json!([0, a, v.0])", "Const(a, v) => json!([0, a, if v.integer_value() == Some(Value::MAX_FIXNUM) { Value::integer(0).unwrap().0 } else { v.0 }])", "fixnums", "widened_code_image_roundtrip", "S109 image integer tags"),
    ("moving-flag-dropped", "src/hash.rs", "!Self::image_value(v) && (matches!(v.tag(), 3..=5) || v.tag() == 0 && v != Value::NIL)", "false && (matches!(v.tag(), 3..=5) || v.tag() == 0 && v != Value::NIL)", "lib", UNIT, "S109 position-hashed classification"),
    ("stale-flag-dropped", "src/hash.rs", "if self.moving[i] && self.keys[i] != old {\n                self.stale = true;\n            }", "if self.moving[i] && self.keys[i] != old {\n                self.stale = false;\n            }", "lib", UNIT, "S109 moving key must invalidate buckets"),
    ("cell-forwarding-dropped", "src/gc.rs", "*v = Value::reference(i + offset, tag);", "*v = if tag == 5 { Value::TRUE } else { Value::reference(i + offset, tag) };", "lib", UNIT, "S109 cell identity after forwarding"),
    ("legacy-decode-dropped", "src/image.rs", "1 => return Value::integer((word as i64) >> 3)", "1 => return Ok(Value(word))", "gc", "read_only_graph_is_shared_while_worker_heaps_collect", "right: 42"),
]


def run(suite, name, env):
    selector = ["--lib"] if suite == "lib" else ["--test", suite]
    return subprocess.run(["cargo", "test", "-p", "romacs-vm", *selector, name,
                           "--", "--nocapture"], cwd=ROOT, env=env,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          text=True, timeout=180)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", type=Path, required=True)
    args = parser.parse_args()
    args.results.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    for key in ("TMPDIR", "CARGO_TARGET_DIR", "GIT_DIR", "GIT_WORK_TREE"):
        env.pop(key, None)
    env["TMPDIR"] = str(args.results.resolve())
    env.setdefault("SCCACHE_SERVER_PORT", "43109")
    outcomes = []
    for label, file, old, new, suite, test, expected in MUTANTS:
        path = CRATE / file
        original = path.read_text()
        if original.count(old) != 1:
            raise RuntimeError(f"{label}: nonunique mutation anchor")
        baseline = run(suite, test, env)
        (args.results / f"{label}-baseline.log").write_text(baseline.stdout)
        if baseline.returncode or "1 passed" not in baseline.stdout:
            raise RuntimeError(f"{label}: baseline not green")
        try:
            path.write_text(original.replace(old, new))
            mutant = run(suite, test, env)
        finally:
            path.write_text(original)
        (args.results / f"{label}.log").write_text(mutant.stdout)
        killed = (mutant.returncode == 101 and "panicked at" in mutant.stdout
                  and "test result: FAILED" in mutant.stdout and expected in mutant.stdout)
        outcomes.append(dict(mutant=label, file=file, old=old, new=new, test=test,
                             baseline_exit=baseline.returncode, exit=mutant.returncode,
                             expected_assertion=expected, killed_at_target=killed))
        (args.results / "results.json").write_text(json.dumps(outcomes, indent=2) + "\n")
        print(f"{label}: {'KILLED AT TARGET' if killed else 'NOT PROVEN'}", flush=True)
        if not killed:
            raise RuntimeError(f"{label}: did not reach target assertion")
    print(f"{len(outcomes)}/{len(MUTANTS)} mutants killed at their target assertions")


if __name__ == "__main__":
    main()
