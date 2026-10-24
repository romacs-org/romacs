#!/usr/bin/env python3
"""Replay real ingress mutants in a separate, already-patched worktree.

Usage: python crates/romacs-display-runtime/scripts/paint_mutants.py WORKTREE
The caller provides an idle disposable checkout containing this candidate. This
script never mutates the checkout it is invoked from. It restores every mutant
in finally, requires the baseline to pass, and rejects compile failures as kills.
Source scripts/lane-env.sh s115 first and use a private sccache port/target.
"""
import os
from pathlib import Path
import subprocess
import sys

root = Path(sys.argv[1]).resolve()
source_root = Path(__file__).resolve().parents[3]
if root == source_root:
    raise SystemExit("refusing to mutate the active shared worktree")
target = root / "crates/romacs-display-runtime/src/render_thread/frame_ingest.rs"
original = target.read_text()
command = ["cargo", "test", "-p", "romacs-display-runtime", "--no-default-features",
           "--features", "core-free", "--lib",
           "core_free_paint_command_installs_and_replaces_scene", "-j", "2", "--", "--exact"]
# Match the fully qualified name when --exact is used.
command[command.index("core_free_paint_command_installs_and_replaces_scene")] = (
    "render_thread::tests::core_free_paint_command_installs_and_replaces_scene")
env = os.environ.copy()
for key in ("TMPDIR", "GIT_DIR", "GIT_WORK_TREE"):
    env.pop(key, None)


def run():
    result = subprocess.run(command, cwd=root, env=env, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    return result.returncode, result.stdout


code, output = run()
if code or "1 passed; 0 failed" not in output:
    print(output)
    raise SystemExit("baseline did not execute and pass the target assertion")

needle = """        self.ingest_primary_paint(
            frame,
            Default::default(),
            Default::default(),
            Default::default(),
        );"""
mutants = {
    "drop-paint": (needle, "        let _ = frame;",
                   "Seam P command must install a frame"),
    "drop-font-projection": (needle, "        let mut frame = frame;\n        frame.char_fonts.clear();\n" + needle,
                             "font projection must survive the host ingress"),
    "ignore-mutation": (needle, "        if current.is_some() { return; }\n" + needle,
                        "mutation must replace the scene"),
}
try:
    for name, (before, after, assertion) in mutants.items():
        if original.count(before) != 1:
            raise SystemExit(f"{name}: candidate changed; mutation target must match once")
        target.write_text(original.replace(before, after))
        code, output = run()
        if code != 101 or assertion not in output or "test result: FAILED" not in output:
            print(output)
            raise SystemExit(f"{name}: survived, failed to compile, or failed off-target")
        print(f"KILLED {name}: {assertion}")
        target.write_text(original)
finally:
    target.write_text(original)
