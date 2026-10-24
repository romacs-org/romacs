#!/usr/bin/env python3
"""Sequential, source-restoring frame contract mutants; run via frame-env.sh.

Writes only the two explicitly authorized source files and a fresh evidence
output directory. Every mutation restores the exact original bytes in finally.
Classification requires compilation, exactly one executed test and its intended
assertion diagnostic; a generic nonzero exit is not a qualified kill.
"""
import os as _portable_os
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[4]
CRATE = ROOT / "crates/romacs-vm"
SHELL = Path(_portable_os.environ['ROMACS_MUTATION_RUNNER'])
PAIRS = [
    ("framep", "framep_gnu_contract"),
    ("frame_list", "frame_list_gnu_contract"),
    ("frame_terminal", "frame_terminal_gnu_contract"),
    ("terminal_live_p", "terminal_live_p_gnu_contract"),
    ("last_nonminibuffer_frame", "last_nonminibuffer_frame_gnu_contract"),
    ("terminal_list", "terminal_list_gnu_contract"),
    ("tty_top_frame", "tty_top_frame_initial_is_not_a_tty"),
    ("window_list", "window_list_gnu_contract"),
    ("get_buffer_window", "get_buffer_window_gnu_contract"),
    ("window_combination_limit", "window_combination_limit_gnu_contract"),
]

def sha(data):
    return hashlib.sha256(data).hexdigest()

def run(command):
    proc = subprocess.run(["bash", str(SHELL), *command], cwd=ROOT,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    return proc.returncode, proc.stdout

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True,
                        help="New directory, refused if already present")
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    files = [CRATE / "src/prims/frame.rs", CRATE / "src/gc.rs"]
    originals = {p: p.read_bytes() for p in files}
    source_manifest = {str(p.relative_to(ROOT)): sha(b) for p, b in originals.items()}
    # Include all tracked source/test inputs, including pre-existing dirty bytes.
    _, names = run(["git", "ls-files", "crates/romacs-vm"])
    inputs = {name: sha((ROOT / name).read_bytes())
              for name in names.decode().splitlines()
              if (ROOT / name).is_file() and "/evidence/" not in name}
    head = run(["git", "rev-parse", "HEAD"])[1].decode().strip()
    before_status = run(["git", "status", "--short"])[1].decode()
    receipt = {"head": head, "source_sha256": source_manifest,
               "tracked_input_sha256": inputs, "status_before": before_status,
               "driver_sha256": sha(Path(__file__).read_bytes()),
               "environment_wrapper_sha256": sha(SHELL.read_bytes()),
               "mutants": [], "restored_controls": []}
    def save():
        (out / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    mutants = []
    frame = originals[files[0]].decode()
    for function, test in PAIRS:
        pattern = r"(fn " + function + r"\([^\n]*?\) \{)(.*?)(\}\})"
        matches = list(re.finditer(pattern, frame, re.S))
        if len(matches) != 1:
            raise RuntimeError(f"non-unique function {function}: {len(matches)}")
        m = matches[0]
        value = "TRUE" if function == "tty_top_frame" else "NIL"
        replacement = f" Ok(Value::{value}) "
        changed = frame[:m.start(2)] + replacement + frame[m.end(2):]
        mutants.append((function, files[0], changed.encode(),
                        ["--test", "frame_prims", test, "--", "--exact", "--nocapture"],
                        f"{function.replace('_', '-')}: ", "GNU contract failures"))
    gc = originals[files[1]].decode()
    for kind, label, diagnostic in [
        ("Terminal", "terminal_backlink", "terminal frame edge must be forwarded"),
        ("Window", "window_frame_backlink", "window backlink must be forwarded"),
    ]:
        var = "t" if kind == "Terminal" else "w"
        old = f"data::Object::{kind}({var}) => {var}.visit_values(&mut |v| rewrite(v)),"
        new = (f"data::Object::{kind}({var}) => {{ let old_frame = {var}.frame; "
               f"{var}.visit_values(&mut |v| rewrite(v)); {var}.frame = old_frame; }},")
        if gc.count(old) != 1:
            raise RuntimeError(f"non-unique forwarding branch {kind}")
        mutants.append((label, files[1], gc.replace(old, new).encode(),
                        ["--lib", "window_tests::frame_graph_edges_move_and_retain_constant_storage",
                         "--", "--exact", "--nocapture"], diagnostic, diagnostic))
    save()
    try:
        for label, path, changed, selection, diagnostic, assertion in mutants:
            for p, b in originals.items():
                if p.read_bytes() != b:
                    raise RuntimeError(f"baseline changed: {p}")
            import difflib
            patch = "".join(difflib.unified_diff(
                originals[path].decode().splitlines(True), changed.decode().splitlines(True),
                fromfile=str(path.relative_to(ROOT)), tofile=str(path.relative_to(ROOT))))
            (out / f"{label}.patch").write_text(patch)
            command = ["cargo", "test", "-p", "romacs-vm", *selection]
            row = {"name": label, "file": str(path.relative_to(ROOT)),
                   "mutated_sha256": sha(changed), "command": ["bash", str(SHELL), *command],
                   "expected_diagnostic": diagnostic, "expected_assertion": assertion}
            try:
                path.write_bytes(changed)
                code, log = run(command)
                (out / f"{label}.log").write_bytes(log)
                text = log.decode(errors="replace")
                row.update(exit_code=code,
                           compilation_success=bool(re.search(r"Finished `test` profile", text)),
                           executed_one="running 1 test" in text,
                           intended_assertion=("panicked at" in text and diagnostic in text and assertion in text),
                           test_failure="test result: FAILED. 0 passed; 1 failed;" in text)
                row["qualified_kill"] = code != 0 and all(row[k] for k in (
                    "compilation_success", "executed_one", "intended_assertion", "test_failure"))
                row["log_sha256"] = sha(log)
            finally:
                path.write_bytes(originals[path])
                row["restored_sha256"] = sha(path.read_bytes())
                row["restored_exactly"] = path.read_bytes() == originals[path]
                receipt["mutants"].append(row)
                save()
            print(label, "QUALIFIED" if row["qualified_kill"] else "UNQUALIFIED", flush=True)
        for label, selection in [
            ("restored_frame_prims", ["--test", "frame_prims", "--", "--nocapture"]),
            ("restored_frame_graph", ["--lib", "window_tests::frame_graph_edges_move_and_retain_constant_storage", "--", "--exact", "--nocapture"]),
        ]:
            command = ["cargo", "test", "-p", "romacs-vm", *selection]
            code, log = run(command)
            (out / f"{label}.log").write_bytes(log)
            text = log.decode(errors="replace")
            expected = 11 if label == "restored_frame_prims" else 1
            passed = code == 0 and f"test result: ok. {expected} passed; 0 failed;" in text
            receipt["restored_controls"].append({"name": label, "command": ["bash", str(SHELL), *command],
                "exit_code": code, "passed": passed, "expected_tests": expected, "log_sha256": sha(log)})
            save()
            print(label, "PASS" if passed else "FAIL", flush=True)
    finally:
        for p, b in originals.items():
            p.write_bytes(b)
        receipt["all_sources_restored_exactly"] = all(p.read_bytes() == b for p, b in originals.items())
        receipt["tracked_inputs_unchanged"] = all((ROOT / p).is_file() and sha((ROOT / p).read_bytes()) == h for p, h in inputs.items())
        receipt["status_after"] = run(["git", "status", "--short"])[1].decode()
        receipt["qualified_kills"] = sum(r.get("qualified_kill", False) for r in receipt["mutants"])
        receipt["complete"] = (len(receipt["mutants"]) == 12 and receipt["qualified_kills"] == 12
            and len(receipt["restored_controls"]) == 2 and all(r["passed"] for r in receipt["restored_controls"])
            and receipt["all_sources_restored_exactly"] and receipt["tracked_inputs_unchanged"])
        save()
    return 0 if receipt["complete"] else 1

if __name__ == "__main__":
    sys.exit(main())
