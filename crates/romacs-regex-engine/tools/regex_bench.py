#!/usr/bin/env python3
"""Same-operation benchmark of the regexp engine: romacs (any number of
`regex_probe` binaries, e.g. master and candidate) against GNU 32.0.50.

The operation, on every side: compile PATTERN once, then search the subject
for it ITERS times -- one search, or every match the way a font-lock loop
does (restart at the match end, one past it after an empty match).  GNU runs
it as a byte-compiled lambda calling `string-match` on a multibyte string
under the standard syntax table, `case-fold-search` nil; romacs runs the
engine alone (`regex_probe bench`) over the same UTF-8 bytes.  Both print a
checksum of the match positions, which must agree, so both did the same work.

Two sets, answering different questions:

* ordinary -- real text, both sides finish.  `perf stat -r REPEAT -e
  task-clock` on the whole process, for ITERS and for 0 iterations; the
  difference over ITERS is the per-operation time, so process start, subject
  loading and compilation cancel.  Run it on a quiet machine: the script
  prints the load average and the rustc count first.
* pathological -- patterns GNU's backtracker does not finish.  One search,
  no repeats: each side completes within `--gnu-budget` seconds or is
  recorded as `timeout@BUDGET` (the datum, not a duration).  romacs is timed
  by one perf run minus its 0-iteration run.  GC: GNU runs the
loop with `gc-cons-threshold` at `most-positive-fixnum` (no collection in
the timed loop); the romacs engine has no Lisp heap.  Never measured through
scripts/measure.

    tools/regex_bench.py --probe master=PATH --probe cand=PATH [--set ordinary|pathological]
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]


def org_with_greek():
    """org.el with `α` opening every 40th line and `ω` closing lines 0 and 79
    of every 80: the owner's `α.*ω` / `α.*?ω` shape over real text -- most
    lines match neither, half the `α` lines match, and the `ω` lines without
    an `α` do not."""
    lines = (ROOT / "lisp/org/org.el").read_text().splitlines()
    out = []
    for i, line in enumerate(lines):
        if i % 40 == 0:
            line = "α " + line
        if i % 80 in (0, 79):
            line = line + " ω"
        out.append(line)
    return "\n".join(out) + "\n"


def simple_el():
    return (ROOT / "lisp/simple.el").read_text()


# (name, kind, pattern, subject thunk, all matches?, iters)
CASES = [
    # Ordinary: real text, every match.
    ("alpha-omega", "ordinary", "α.*ω", org_with_greek, True, 20),
    ("alpha-omega-lazy", "ordinary", "α.*?ω", org_with_greek, True, 20),
    ("defun", "ordinary", r"^(def\(?:un\|var\|custom\)\s-+\(\(?:\sw\|\s_\)+\)", simple_el, True, 20),
    ("symbol", "ordinary", r"\_<\(?:let\*?\|when\|unless\)\_>", simple_el, True, 10),
    ("string-lit", "ordinary", r"\"\(?:[^\"\\]\|\\.\)*\"", simple_el, True, 10),
    ("number", "ordinary", r"[0-9]+\.[0-9]+", simple_el, True, 20),
    ("comment", "ordinary", r";;+ \(.*\)$", simple_el, True, 10),
    ("literal", "ordinary", "save-excursion", simple_el, True, 50),
    ("word-class", "ordinary", r"\(\sw+\)-\(\sw+\)", simple_el, True, 5),
    # Pathological: GNU-feasible sizes, then sizes where GNU does not finish.
    ("a3-25", "pathological", "a*a*a*b", lambda: "a" * 25, False, 2000),
    ("a3-20k", "pathological", "a*a*a*b", lambda: "a" * 20000, False, 2),
    ("a3-100k", "pathological", "a*a*a*b", lambda: "a" * 100000, False, 1),
    ("astarstar-22", "pathological", r"\(a*\)*b", lambda: "a" * 22, False, 3),
    ("astarstar-20k", "pathological", r"\(a*\)*b", lambda: "a" * 20000, False, 2),
    ("alt-aa-24", "pathological", r"\(?:a\|aa\)*b", lambda: "a" * 24, False, 20),
    ("alt-aa-20k", "pathological", r"\(?:a\|aa\)*b", lambda: "a" * 20000, False, 2),
    ("xpp-22", "pathological", r"\(?:x+x+\)+y", lambda: "x" * 22, False, 3),
    ("xpp-20k", "pathological", r"\(?:x+x+\)+y", lambda: "x" * 20000, False, 2),
    ("dotdot-400", "pathological", r".*.*=.*;", lambda: "x" * 400, False, 20),
    ("dotdot-20k", "pathological", r".*.*=.*;", lambda: "x" * 20000, False, 2),
    ("long-nomatch", "pathological", r"[a-z]+@[a-z]+\.org",
     lambda: "lorem ipsum dolor sit amet consectetur " * 5000, False, 20),
]

GNU_PROGRAM = r'''
(set-language-environment "UTF-8")
(let* ((s (with-temp-buffer (insert-file-contents SUBJECT) (buffer-string)))
       (fn (byte-compile
            (lambda (re s iters all)
              (with-syntax-table (standard-syntax-table)
                (let ((case-fold-search nil) (gc-cons-threshold most-positive-fixnum)
                      (sum 0) (matches 0))
                  (dotimes (_ iters)
                    (let ((start 0))
                      (while (and start (string-match re s start))
                        (let ((b (match-beginning 0)) (e (match-end 0)))
                          (setq matches (1+ matches))
                          (setq sum (logand (+ (* sum 31) (* b 7) e) #xFFFFFFFFFFFFFFFF))
                          (setq start (and all (if (> e b) e (1+ e))))
                          (when (and start (> start (length s))) (setq start nil))))))
                  (format "matches=%d checksum=%d" matches sum)))))))
  (princ (funcall fn PATTERN s ITERS ALL)))
'''


def perf_ms(cmd, repeat, env=None, timeout=600):
    """Mean task-clock milliseconds of CMD over REPEAT runs, and its stdout."""
    ms, _, out = perf_stat(cmd, repeat, env, timeout)
    return ms, out


def perf_stat(cmd, repeat, env=None, timeout=600):
    """Mean task-clock ms and user-space instructions of CMD over REPEAT runs,
    and its stdout.  Instructions barely move under CPU contention, so they
    are the figure to trust when the machine is busy."""
    out = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout)
    if out.returncode != 0:
        raise RuntimeError(f"{cmd[0]} failed: {out.stderr[-500:]}")
    stat = subprocess.run(["perf", "stat", "-x", ",", "-e", "task-clock,instructions:u",
                           "-r", str(repeat), "--"] + cmd, capture_output=True, text=True,
                          env=env, timeout=timeout * repeat)
    ms = instructions = None
    for line in stat.stderr.splitlines():
        fields = line.split(",")
        if len(fields) > 2 and fields[2].startswith("task-clock"):
            ms = float(fields[0])
        if len(fields) > 2 and fields[2].startswith("instructions"):
            instructions = float(fields[0])
    if ms is None or instructions is None:
        raise RuntimeError(f"no task-clock/instructions in perf output: {stat.stderr[-500:]}")
    return ms, instructions, out.stdout.strip()


def gnu_cmd(gnu, pattern, subject_file, iters, all_matches):
    program = (GNU_PROGRAM.replace("SUBJECT", json.dumps(str(subject_file)))
               .replace("PATTERN", json.dumps(pattern, ensure_ascii=False))
               .replace("ITERS", str(iters)).replace("ALL", "t" if all_matches else "nil"))
    return [gnu, "-Q", "--batch", "--eval", f"(progn {program})"]


def run_once_seconds(cmd, env, budget):
    """One run of CMD under a wall-clock budget: (seconds, stdout) or None on
    timeout.  For the pathological set, where GNU's timeout is the datum."""
    import time
    t0 = time.monotonic()
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=budget)
    except subprocess.TimeoutExpired:
        return None
    if out.returncode != 0:
        raise RuntimeError(f"{cmd[0]} failed: {out.stderr[-500:]}")
    return time.monotonic() - t0, out.stdout.strip()


def ordinary_row(args, probes, env, name, kind, pattern, subject_file, all_matches, iters):
    row: dict[str, object] = dict(case=name, kind=kind, pattern=pattern, iters=iters,
                                  subject_chars=len(subject_file.read_text()))
    checks = set()
    flag = "1" if all_matches else "0"
    sides = [(label, [path, "bench", pattern, str(subject_file), str(iters), flag],
              [path, "bench", pattern, str(subject_file), "0", flag], None)
             for label, path in probes]
    sides.append(("gnu", gnu_cmd(args.gnu, pattern, subject_file, iters, all_matches),
                  gnu_cmd(args.gnu, pattern, subject_file, 0, all_matches), env))
    for label, full_cmd, base_cmd, side_env in sides:
        full_ms, full_ins, out = perf_stat(full_cmd, args.repeat, side_env)
        base_ms, base_ins, _ = perf_stat(base_cmd, args.repeat, side_env)
        row[label] = round((full_ms - base_ms) / iters, 4)
        row[label + "_minstr"] = round((full_ins - base_ins) / iters / 1e6, 3)
        checks.add(out)
    row["agree"] = len(checks) == 1
    row["checksum"] = sorted(checks)
    return row


def pathological_row(args, probes, env, name, kind, pattern, subject_file):
    """One search, no repeats: each side completes within the budget or does
    not.  romacs is timed by perf (task-clock of one search minus the
    0-iteration run); GNU by wall clock against --gnu-budget, killed at it."""
    row: dict[str, object] = dict(case=name, kind=kind, pattern=pattern, iters=1,
                                  subject_chars=len(subject_file.read_text()))
    checks = set()
    for label, path in probes:
        cmd = [path, "bench", pattern, str(subject_file), "1", "0"]
        once = run_once_seconds(cmd, None, args.gnu_budget)
        if once is None:
            row[label] = f"timeout@{args.gnu_budget:g}s"
            continue
        full, out = perf_ms(cmd, 1)
        base, _ = perf_ms([path, "bench", pattern, str(subject_file), "0", "0"], 1)
        row[label] = round(full - base, 4)
        checks.add(out)
    once = run_once_seconds(gnu_cmd(args.gnu, pattern, subject_file, 1, False), env,
                            args.gnu_budget)
    if once is None:
        row["gnu"] = f"timeout@{args.gnu_budget:g}s"
    else:
        row["gnu"] = f"completed in {once[0]:.3f}s wall (incl. startup)"
        checks.add(once[1])
    row["agree"] = len(checks) <= 1
    row["checksum"] = sorted(checks)
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--probe", action="append", required=True, help="NAME=PATH")
    parser.add_argument("--gnu", default="/usr/bin/emacs")
    parser.add_argument("--set", choices=["ordinary", "pathological", "both"], default="both")
    parser.add_argument("--gnu-budget", type=float, default=2.0,
                        help="pathological set: seconds any side may take before it is "
                             "recorded as timeout@BUDGET")
    parser.add_argument("--repeat", type=int, default=5, help="ordinary set: perf -r")
    parser.add_argument("--only", nargs="*")
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    probes = [p.split("=", 1) for p in args.probe]
    home = Path(tempfile.mkdtemp(prefix="regex-bench-"))
    env = dict(PATH="/usr/bin:/bin", LANG="C.UTF-8", HOME=str(home), TMPDIR=str(home))
    load = os.getloadavg()
    rustc = subprocess.run(["pgrep", "-x", "-c", "rustc"], capture_output=True,
                           text=True).stdout.strip()
    print(json.dumps(dict(loadavg=load, rustc_processes=rustc)), flush=True)
    rows = []
    for name, kind, pattern, subject, all_matches, iters in CASES:
        if args.only and name not in args.only:
            continue
        if args.set != "both" and kind != args.set:
            continue
        subject_file = home / f"{name}.txt"
        subject_file.write_text(subject())
        if kind == "ordinary":
            row = ordinary_row(args, probes, env, name, kind, pattern, subject_file,
                               all_matches, iters)
        else:
            row = pathological_row(args, probes, env, name, kind, pattern, subject_file)
        rows.append(row)
        print(json.dumps(row, ensure_ascii=False), flush=True)
    if args.json:
        args.json.write_text(json.dumps(dict(
            operation="compile once; ITERS x (one search | every match), same subject bytes",
            ordinary="perf stat task-clock (ms) and instructions:u (millions), mean over --repeat, "
                     "(ITERS run - 0-iteration run) / ITERS",
            pathological="one search; romacs perf task-clock ms (1 run minus 0-iteration run); "
                         "GNU completion or timeout@--gnu-budget",
            gc="GNU gc-cons-threshold most-positive-fixnum in the loop; romacs engine has no GC",
            loadavg=load, rustc_processes=rustc, rows=rows), ensure_ascii=False, indent=1) + "\n")


if __name__ == "__main__":
    main()
