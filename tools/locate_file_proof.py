#!/usr/bin/env python3
"""Focused red/green and assertion-message-keyed production mutation proof.
Run in an owned isolated worktree with the prepared project environment.
"""
import json
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = ROOT / "crates/romacs-vm/src/prims/fileio.rs"
CASES = [
    ("runtime_root_resolves_gnu_lisp_before_bundled_lisp", "locate-file: runtime load-path must resolve GNU before bundled Lisp"),
    ("loadup_load_path_finds_every_gnu_file_before_bundled_lisp", "locate-file: GNU locate-library must resolve GNU before bundled Lisp after loadup"),
    ("loadup_load_path_reaches_gnu_subdirectories_in_gnu_order", "Org is not findable after loadup: load-path"),
]


def main():
    branch = subprocess.check_output(["git", "branch", "--show-current"], cwd=ROOT, text=True).strip()
    if branch in ("master", "main", ""):
        raise SystemExit("proof requires an isolated non-master branch")
    original = SOURCE.read_text()
    start = original.index("// Cold bootstrap needs locate-file")
    end = original.index("/// Open PATH for `insert-file-contents'", start)
    missing = original[:start] + original[end:]
    needle = "if std::path::Path::new(&candidate).is_file() && open_for_reading(&candidate).is_ok() {"
    assert original.count(needle) == 1
    mutant = original.replace(needle, "if false && std::path::Path::new(&candidate).is_file() && open_for_reading(&candidate).is_ok() {")
    rows = []
    try:
        for phase, source in [("missing-production-red", missing), ("green", original), ("never-find-production-mutant", mutant), ("restored-green", original)]:
            SOURCE.write_text(source)
            for name, message in CASES:
                cmd = ["cargo", "test", "-p", "romacs-vm-gui", "--bin", "romacs-vm-gui", name, "--", "--nocapture"]
                p = subprocess.run(cmd, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                red = phase in ("missing-production-red", "never-find-production-mutant")
                assertion = "assertion `left == right` failed: " + message
                good = p.returncode != 0 and assertion in p.stdout and "1 failed;" in p.stdout if red else p.returncode == 0 and "1 passed;" in p.stdout
                at = p.stdout.find(assertion)
                rows.append({"phase": phase, "test": name, "command": cmd, "returncode": p.returncode,
                             "assertion_message": message, "verified": good,
                             "failure_excerpt": p.stdout[at:at + 1400] if at >= 0 else None})
                print(f"{phase}: {name}: rc={p.returncode}, verified={good}", flush=True)
                if not good:
                    print(p.stdout)
                    raise RuntimeError("focused proof failed, not an assertion-message-keyed kill")
    finally:
        SOURCE.write_text(original)
        out = ROOT / "crates/romacs-vm/evidence/locate-file-proof.json"
        out.write_text(json.dumps({"branch": branch, "rows": rows}, indent=2) + "\n")
    print("All missing-production reds, production mutant kills, and restored greens verified.")


if __name__ == "__main__":
    main()
