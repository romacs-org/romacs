#!/usr/bin/env python3
"""Single-site mutants for the window-prims lane (vm2/window-prims).

Each mutant must apply exactly once, compile, and fail the named test AT ITS
TARGET ASSERTION: the panic location must be the named line in the named test
file and the panic message must contain the marker. A compile failure, a panic
anywhere else (a collector invariant, an unwrap, a recursion abort) or a pass is
NOT KILLED. Run from the repository root through lane-run. Restores every file.
"""
from pathlib import Path as _PortablePath
import re, subprocess, sys
from pathlib import Path

ROOT = Path.cwd()
W = "crates/romacs-vm/src/window.rs"
PW = "crates/romacs-vm/src/prims/window.rs"
R = "crates/romacs-vm/src/prims/rounding.rs"
PT = "crates/romacs-vm/tests/primh_window.rs"
GT = "crates/romacs-vm/tests/window_prims.rs"


def line_of(path, marker):
    for i, l in enumerate((ROOT / path).read_text().splitlines(), 1):
        if marker in l:
            return i
    raise SystemExit(f"no line {marker!r} in {path}")


CHECK = (PT, line_of(PT, "assert!(bad.is_empty()"))
CONTENT = (GT, line_of(GT, '"{setup}: contents after a moving collection"'))
FREED = (GT, line_of(GT, '"{setup}: objects live only through the window"'))
PARENT = (GT, line_of(GT, '"parent normal sizes after a moving collection"') - 3)
RESET = (GT, line_of(GT, '"split window\'s normal sizes reset by the second split"'))
INHERIT = (GT, line_of(GT, '"normal sizes inherited from the root"'))

MUTANTS = [
    # name, file, old, new, test binary, test name, (file, line), marker
    ("trace-new-normal", W, "            &mut self.new_normal,\n            &mut self.normal_lines,", "            &mut self.normal_lines,",
     "window_prims", "new_normal_slot_retains_its_object_across_collection", CONTENT, "contents after a moving collection"),
    ("trace-normal-lines", W, "            &mut self.normal_lines,\n            &mut self.normal_cols,", "            &mut self.normal_cols,",
     "window_prims", "normal_lines_slot_retains_its_float_across_collection", CONTENT, "contents after a moving collection"),
    ("trace-normal-cols/parent", W, "            &mut self.normal_cols,\n        ] {", "        ] {",
     "window_prims", "split_parent_keeps_inherited_normal_sizes_across_collection", PARENT, "parent normal sizes"),
    ("trace-normal-cols", W, "            &mut self.normal_cols,\n        ] {", "        ] {",
     "window_prims", "normal_cols_slot_retains_its_float_across_collection", CONTENT, "contents after a moving collection"),
    ("split-keeps-window-normals", W, "                old.normal_lines = lines;\n                old.normal_cols = cols;\n", "                let _ = (old, lines, cols);\n",
     "window_prims", "split_hands_normal_sizes_to_the_parent_and_resets_the_window", RESET, "reset by the second split"),
    ("delete-drops-root-normals", W, "            x.normal_lines = normal_lines;\n            x.normal_cols = normal_cols;\n", "            let _ = (x, normal_lines, normal_cols);\n",
     "window_prims", "delete_other_windows_internal_inherits_the_roots_normal_sizes", INHERIT, "inherited from the root"),
    ("new-normal-not-cleared", W, "        self.get_mut(w)?.new_normal = size;\n        Ok(size)", "        if size != Value::NIL { self.get_mut(w)?.new_normal = size; }\n        Ok(size)",
     "window_prims", "new_normal_slot_retains_its_object_across_collection", FREED, "objects live only through the window"),
    ("top-child-is-left-child", PW, "    cx.windows().first_child(window, false)", "    cx.windows().first_child(window, true)",
     "primh_window", "primh_window_top_child", CHECK, "window-top-child"),
    ("use-time-not-bumped", W, "            self.window_state.select_count += 1;\n            let count = self.window_state.select_count;", "            let count = self.window_state.select_count;",
     "primh_window", "primh_window_use_time", CHECK, "window-use-time"),
    ("new-total-add-ignored", W, "        let total = if add == Value::NIL {", "        let total = if true {",
     "primh_window", "primh_set_window_new_total", CHECK, "set-window-new-total"),
    ("explicit-window-all-frames", W, "        } else {\n            Value::NIL\n        };", "        } else {\n            all_frames\n        };",
     "primh_window", "primh_next_window", CHECK, "next-window"),
    ("explicit-window-all-frames/list", W, "        } else {\n            Value::NIL\n        };", "        } else {\n            all_frames\n        };",
     "primh_window", "primh_window_list_1", CHECK, "window-list-1"),
    ("delete-keeps-own-normal", W, "            s.normal_cols = cols;\n            s.normal_lines = lines;", "            let _ = (cols, lines, s);",
     "primh_window", "primh_window_normal_size", CHECK, "window-normal-size"),
    ("scroll-hook-gets-decoded-window", PW, "    let (_, buffer, start) = cx.windows().scroll_functions_call(window)?;", "    let (window, buffer, start) = cx.windows().scroll_functions_call(window)?;",
     "primh_window", "primh_run_window_scroll_functions", CHECK, "run-window-scroll-functions"),
    ("mode-line-none-ignored", W, "            && parameter != none\n", "            && (parameter != none || true)\n",
     "primh_window", "primh_window_mode_line_height", CHECK, "window-mode-line-height"),
    ("minibuffer-error-straight-quote", W, "Can’t expand minibuffer to full frame", "Can't expand minibuffer to full frame",
     "primh_window", "primh_delete_other_windows_internal", CHECK, "delete-other-windows-internal"),
    ("round-ties-away", R, "x.round_ties_even()", "x.round()",
     "primh_window", "primh_round", CHECK, "round"),
    ("round-exact-tie-always-up", R, "            std::cmp::Ordering::Equal => !r_zero && q & 1 == 1,", "            std::cmp::Ordering::Equal => !r_zero,",
     "primh_window", "primh_round", CHECK, "round"),
    ("ceiling-fixnum-ignores-sign", R, "Mode::Ceiling => q + i128::from(r != 0 && (n < 0) == (d < 0)),", "Mode::Ceiling => q + i128::from(r != 0),",
     "primh_window", "primh_ceiling", CHECK, "ceiling"),
    ("ceiling-float-truncates", R, "if mode == Mode::Round { x.round_ties_even() } else { x.ceil() }", "if mode == Mode::Round { x.round_ties_even() } else { x.trunc() }",
     "primh_window", "primh_ceiling", CHECK, "ceiling"),
    ("live-check-is-valid-check", W, "    pub(crate) fn window_use_time(&mut self, window: Value) -> Out {\n        let w = self.live(window)?;", "    pub(crate) fn window_use_time(&mut self, window: Value) -> Out {\n        let w = self.valid(window)?;",
     "primh_window", "primh_window_use_time", CHECK, "window-use-time"),
]

LR = str(next(p for p in _PortablePath(__file__).resolve().parents if (p / 'parity-reference.toml').is_file()) / 'scripts/lane-run')
LANE = sys.argv[1] if len(sys.argv) > 1 else "win"
only = set(sys.argv[2:])
# A typo in a selector used to select nothing and report 0/0 with exit 0.
unknown = only - {m[0] for m in MUTANTS}
if unknown:
    raise SystemExit(f"unknown mutant(s): {sorted(unknown)}")
rows = []
for name, path, old, new, binary, test, (tfile, tline), marker in MUTANTS:
    if only and name not in only:
        continue
    p = ROOT / path
    text = p.read_text()
    if text.count(old) != 1:
        rows.append((name, f"NOT APPLIED ({text.count(old)} sites)"))
        print(rows[-1], flush=True)
        continue
    p.write_text(text.replace(old, new))
    try:
        r = subprocess.run([LR, LANE, "-C", str(ROOT), "--", "cargo", "test", "-p", "romacs-vm", "--test", binary, "--", test, "--exact"],
                           capture_output=True, text=True, timeout=1500)
        out = r.stdout + r.stderr
        log = ROOT / "crates/romacs-vm/tools/primh/logs/window-prims" / ("mutant-" + name.replace("/", "_") + ".log")
        log.parent.mkdir(parents=True, exist_ok=True)
        log.write_text(f"rc={r.returncode}\n" + out)
        panics = re.findall(r"panicked at ([^:\n]+):(\d+):\d+:\n(.*)", out)
        # Only rustc's own refusal is a compile error. A failing test makes
        # cargo print `error: test failed, to rerun pass ...`, which the old
        # `^error:` pattern scored as a compile error, so every kill read
        # NOT KILLED (compile error).
        if "could not compile" in out or re.search(r"^error\[E\d+\]", out, re.M):
            verdict = "NOT KILLED (compile error)"
        elif r.returncode == 0:
            verdict = "SURVIVED"
        elif any(f == tfile and int(l) == tline and marker in msg for f, l, msg in panics):
            verdict = f"KILLED at {tfile}:{tline}"
        else:
            verdict = f"NOT KILLED (died elsewhere: {panics[:2] or out[-300:]})"
    finally:
        p.write_text(text)
    rows.append((name, verdict))
    print(f"{name:36} {test:60} {verdict}", flush=True)
killed = sum(v.startswith("KILLED") for _, v in rows)
print(f"\n{killed}/{len(rows)} killed at the target assertion")
sys.exit(0 if rows and killed == len(rows) else 1)
