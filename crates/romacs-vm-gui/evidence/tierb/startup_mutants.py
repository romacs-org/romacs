#!/usr/bin/env python3
"""Mutants for the merge resolution of gui/tierb with gui/loadup's workspace():
GNU's mode-line-format wins when loadup set it; otherwise each workspace buffer
gets its own labelled stand-in. Apply one mutant at a time, run the startup
tests, restore. A kill counts only if the named target test FAILED by an
assertion panic (a compile error is not a kill).

    python3 startup_mutants.py OUT.json
"""
import json, pathlib, subprocess, sys
root = pathlib.Path(__file__).resolve().parents[4]
F = 'crates/romacs-vm-gui/src/startup.rs'
MUTANTS = [
    # (name, old, new, target test)
    ('gnu-format-overwritten', 'let mode_line = if started.gnu_mode_line { "nil" } else { "t" };',
     'let mode_line = "t";', 'workspace_leaves_gnus_mode_line_format_alone_when_loadup_set_it'),
    ('stand-in-unlabelled', '"   " "[stand-in: no bindings.el]")"#;', '"   ")"#;',
     'workspace_mode_lines_are_labelled_stand_ins_when_bindings_el_did_not_load'),
    ('evaluations-not-per-buffer',
     "  (when {mode_line}\n    (make-local-variable 'mode-line-format)\n    (setq mode-line-format (append @STAND_IN_MODE_LINE@ '(\"   evaluated at startup\"))))",
     "  (when {mode_line}\n    (setq mode-line-format (append @STAND_IN_MODE_LINE@ '(\"   evaluated at startup\"))))",
     'workspace_mode_lines_are_labelled_stand_ins_when_bindings_el_did_not_load'),
    ('scratch-not-per-buffer',
     "    (make-local-variable 'mode-line-format)\n    (setq mode-line-format (append @STAND_IN_MODE_LINE@ '(\"   Rum / VM\"))))",
     "    (setq mode-line-format (append @STAND_IN_MODE_LINE@ '(\"   Rum / VM\"))))",
     'startup_publishes_mode_lines_and_exact_rum_faces'),
    ('buffer-tag-dropped', "(append @STAND_IN_MODE_LINE@ '(\"   evaluated at startup\"))",
     "(append @STAND_IN_MODE_LINE@ nil)", 'startup_publishes_mode_lines_and_exact_rum_faces'),
    ('evaluations-local-even-when-gnu-set-it',
     "  (when {mode_line}\n    (make-local-variable 'mode-line-format)\n    (setq mode-line-format (append @STAND_IN_MODE_LINE@ '(\"   evaluated",
     "  (when t\n    (make-local-variable 'mode-line-format)\n    (setq mode-line-format (append @STAND_IN_MODE_LINE@ '(\"   evaluated",
     'workspace_leaves_gnus_mode_line_format_alone_when_loadup_set_it'),
    ('gnu-nil-counts-as-unset', "(default-boundp 'mode-line-format)",
     "(and (default-boundp 'mode-line-format) (default-value 'mode-line-format))",
     'workspace_leaves_a_gnu_set_nil_mode_line_format_alone'),
]
results = []
for name, old, new, target in MUTANTS:
    f = root / F
    src = f.read_text()
    n = src.count(old)
    if n != 1:
        results.append({'mutant': name, 'error': f'pattern matched {n} times'}); print(results[-1], flush=True); continue
    f.write_text(src.replace(old, new))
    assert f.read_text() != src
    try:
        p = subprocess.run(['cargo', 'test', '-j', '6', '-p', 'romacs-vm-gui', '--lib', '--no-fail-fast', '--', 'startup::tests'],
                           cwd=root, capture_output=True, text=True)
        out = p.stdout + p.stderr
        log = pathlib.Path(sys.argv[1]).with_name(f'{pathlib.Path(sys.argv[1]).stem}-{name}.log')
        log.write_text(out)
        failed = [l.split()[1] for l in out.splitlines() if l.startswith('test ') and l.endswith('FAILED')]
        panics = [l for l in out.splitlines() if 'panicked at' in l]
        compile_error = ('error[' in out or 'could not compile' in out) and not failed
        killed = any(t.endswith('::' + target) for t in failed) and not compile_error and bool(panics) and 'assertion' in out
        results.append({'mutant': name, 'target': target, 'killed_at_target': killed, 'failed': failed,
                        'compile_error': compile_error, 'panics': panics[:4], 'log': str(log)})
    finally:
        f.write_text(src)
    print(json.dumps(results[-1]), flush=True)
pathlib.Path(sys.argv[1]).write_text(json.dumps(results, indent=1))
sys.exit(0 if all(r.get('killed_at_target') for r in results) else 1)
