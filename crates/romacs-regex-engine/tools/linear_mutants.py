#!/usr/bin/env python3
"""Mutation proofs for the linear-time matcher's tests (R92 step 2).

Each mutant breaks one thing a test in `src/engine/tests/linear_test.rs` (or an
existing crate test) protects.  For every mutant the script applies the edit
to the working tree, runs the named tests, requires that run to FAIL with the
named test reported FAILED, and restores the file -- so a reviewer can re-run
the whole list:

    tools/linear_mutants.py            # from crates/romacs-regex-engine
    tools/linear_mutants.py M4 M5      # a subset

Requires a clean copy of the files it edits (it refuses otherwise) and the
lane's cargo environment.
"""
import subprocess
import sys
from pathlib import Path

CRATE = Path(__file__).resolve().parents[1]

# (id, file, old, new, test filter, test that must fail, what it breaks)
MUTANTS = [
    ("M1", "src/engine/linear.rs",
     "    Linear::PathSensitive(loops.into_boxed_slice())\n}",
     "    let _ = loops;\n    Linear::Plain\n}",
     "linear_tests", "empty_loop_captures_follow_gnu_in_every_engine",
     "no path-sensitive closure: the empty iteration is pruned by pc"),
    # (Disabling the `mask & bit` exit alone is an equivalent mutant: the
    # fall-through it would add is the node `(pc + 3, mask)` already visited,
    # so the (pc, mask) dedup drops it and the target runs as before.)
    ("M2", "src/engine/linear.rs",
     "scratch.stack.push((pc + 3, mask | bit.unwrap_or(0), caps));",
     "scratch.stack.push((pc + 3, mask, caps));",
     "linear_tests", "empty_loop_captures_follow_gnu_in_every_engine",
     "entering a loop body is not recorded: the empty pass is pruned again"),
    ("M3", "src/engine/linear.rs",
     "                        matched = Some((thread.start, d, thread.caps.clone()));\n                        break;",
     "                        matched = Some((thread.start, d, thread.caps.clone()));",
     "linear_search_agrees_with_the_backtracker", "linear_search_agrees_with_the_backtracker",
     "a match does not cut lower-priority threads (leftmost-first lost)"),
    ("M4", "src/engine/matcher/fail.rs",
     "    if scratch.work > scratch.work_budget {",
     "    if false && scratch.work > scratch.work_budget {",
     "a_failing_scan_does_linear_anchored_work", "a_failing_scan_does_linear_anchored_work",
     "the quadratic-scan guard never hands the search to the linear matcher"),
    ("M5", "src/engine/search.rs",
     "        let span = if scratch.parity_abandoned {\n            usize::MAX\n        } else {\n            free_span\n        };",
     "        let span = { let _ = free_span; usize::MAX };",
     "a_linear_search_keeps_gnu_fail_stack_overflow", "a_linear_search_keeps_gnu_fail_stack_overflow",
     "the linear search never defers a candidate that could overflow GNU's stack"),
    ("M6", "src/engine/matcher/fail.rs",
     "const PARITY_POP_BUDGET: u64 = 4 * FAIL_STACK_ENTRY_LIMIT as u64;",
     "const PARITY_POP_BUDGET: u64 = u64::MAX;",
     "a_parity_run_that_would_freeze_gnu_is_abandoned", "a_parity_run_that_would_freeze_gnu_is_abandoned",
     "the parity backtracker is never abandoned (GNU's freeze returns)"),
    ("M7", "src/engine/matcher/mod.rs",
     "let budgeted = pattern.linear.eligible() && !force_backtrack();",
     "let budgeted = pattern.pike_eligible && !force_backtrack();",
     "a_capture_in_a_nullable_loop_falls_back", "a_capture_in_a_nullable_loop_falls_back_to_the_linear_matcher",
     "a capture in a nullable loop stays on the unbudgeted backtracker"),
    ("M8", "src/engine/search.rs",
     "(pattern.fastmap_accurate && !pattern.can_be_null && pattern.translate.is_none())",
     "(pattern.fastmap_accurate && pattern.translate.is_none())",
     "linear_search_agrees_with_the_backtracker", "linear_search_agrees_with_the_backtracker",
     "the fastmap skips positions where a nullable pattern matches empty"),
    ("M9", "src/engine/linear.rs",
     "((multibyte && (text[d] & 0xC0) == 0x80)",
     "((false && multibyte && (text[d] & 0xC0) == 0x80)",
     "linear_tests", "a_search_from_inside_a_character_starts_at_the_next_one",
     "candidates start inside a multibyte character"),
    ("M10", "src/engine/linear.rs",
     "    regs.start[0] = start as i64;",
     "    regs.start[0] = start.saturating_sub(1) as i64;",
     "linear_tests", "linear_search_agrees_with_the_backtracker",
     "the match start is not the winning thread's candidate"),
    ("M11", "src/engine/linear.rs",
     "            scratch.stack.push((pc + 3, mask | bit.unwrap_or(0), caps));\n",
     "            scratch.stack.push((pc + 3, mask | bit.unwrap_or(0), caps));\n            let top = scratch.stack.len() - 1;\n            scratch.stack.swap(top, top - 1);\n",
     "linear_tests", "linear_anchored_match_agrees_with_the_backtracker",
     "a split explores its jump target before its fall-through (greedy lost)"),
]


def run(mutant):
    ident, rel, old, new, filt, must_fail, what = mutant
    path = CRATE / rel
    original = path.read_text()
    if original.count(old) != 1:
        return ident, "STALE", f"{rel}: expected one occurrence of the mutated text"
    path.write_text(original.replace(old, new))
    try:
        result = subprocess.run(
            ["cargo", "test", "-p", "romacs-regex-engine", "--release", filt],
            cwd=CRATE, capture_output=True, text=True, timeout=900)
        out = result.stdout + result.stderr
    except subprocess.TimeoutExpired:
        return ident, "SURVIVED?", "timed out"
    finally:
        path.write_text(original)
    killed = result.returncode != 0 and f"{must_fail} ... FAILED" in out
    if killed:
        return ident, "KILLED", f"{must_fail} failed: {what}"
    if "error[E" in out:
        return ident, "NO-COMPILE", what
    return ident, "SURVIVED", what


def main():
    wanted = set(sys.argv[1:])
    dirty = subprocess.run(["git", "status", "--porcelain", "--", "src"], cwd=CRATE,
                           capture_output=True, text=True).stdout.strip()
    if dirty:
        sys.exit(f"refusing: uncommitted changes under src/:\n{dirty}")
    rows = [run(m) for m in MUTANTS if not wanted or m[0] in wanted]
    for ident, verdict, note in rows:
        print(f"{ident:4} {verdict:10} {note}")
    sys.exit(0 if all(v == "KILLED" for _, v, _ in rows) else 1)


if __name__ == "__main__":
    main()
