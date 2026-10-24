#!/usr/bin/env python3
"""Apply one source mutant at a time, run the Tier B tests, restore. Each
mutant must turn at least one named test red at its target assertion."""
import os, subprocess, sys, json, pathlib
root = pathlib.Path(os.environ['ROMACS_MUTATION_ROOT'])
ONLY = set(sys.argv[2:])
MUTANTS = [
    ('recenter-no-extra-line', 'crates/romacs-vm/src/redisplay.rs',
     'back += 1;', 'back += 0;'),
    ('recenter-ignores-dy', 'crates/romacs-vm/src/redisplay.rs',
     'let mut back = (dy / line).max(1);', 'let mut back = 1;'),
    ('visible-check-off-by-one', 'crates/romacs-vm/src/redisplay.rs',
     'if row.is_some_and(|row| row < rows) {', 'if row.is_some_and(|row| row + 1 < rows) {'),
    ('force-start-ignored', 'crates/romacs-vm/src/redisplay.rs',
     'if forced && row.is_none_or(|row| row >= displayed) {', 'if false && forced {'),
    ('inactive-face-never-used', 'crates/romacs-vm/src/display.rs',
     'let wanted = if selected || !distinguish {', 'let wanted = if true || selected || !distinguish {'),
    ('percent-wrong-denominator', 'crates/romacs-vm/src/mode_line.rs',
     "format!(\"{:2}%\", percent99(top - begv, zv - begv))", "format!(\"{:2}%\", percent99(top - begv, zv))"),
    ('modiff-not-counted', 'crates/romacs-vm/src/text.rs',
     'self.modiff += u64::from(start != end || !s.is_empty());', 'self.modiff += 0;'),
    ('append-modiff-not-counted', 'crates/romacs-vm/src/text.rs',
     'self.modiff += u64::from(!s.is_empty());', 'self.modiff += 0;'),
    ('percent-uses-window-end', 'crates/romacs-vm/src/mode_line.rs',
     "format!(\"{:2}%\", percent99(top - begv, zv - begv))", "format!(\"{:2}%\", percent99(bottom - begv, zv - begv))"),
    ('band-not-clipped', 'crates/romacs-vm/src/mode_line.rs',
     'max_cols: 4 * state.frame_cols }', 'max_cols: usize::MAX / 2 }'),
    ('one-row-fallback-dropped', 'crates/romacs-vm/src/redisplay.rs',
     'if moved * line >= text_height {', 'if false && moved * line >= text_height {'),
    ('global-format-only', 'crates/romacs-vm/src/display.rs',
     'self.slots[self.buffer_slot].value = self.slots[buffer_slot].value;\n        self.display_state.building_band = true;',
     'self.display_state.building_band = true;'),
    ('propertize-face-dropped', 'crates/romacs-vm/src/mode_line.rs',
     'face_of(&plist)', 'None'),
    ('conditional-takes-then', 'crates/romacs-vm/src/mode_line.rs',
     '                            if truthy {\n                                elt = then;', '                            if true {\n                                elt = then;'),
    ('dashes-short', 'crates/romacs-vm/src/mode_line.rs',
     "{ 4 * state.frame_cols - 1 } else { LOTS_OF_DASHES }", "{ state.frame_cols } else { LOTS_OF_DASHES }"),
    ('percent-e-not-gnus', 'crates/romacs-vm/src/mode_line.rs',
     "'e' => String::new(),", "'e' => \"!MEM FULL! \".into(),"),
    ('object-invalidates-construct', 'crates/romacs-vm/src/mode_line.rs',
     '.or_else(|_| self.expansion_form(value, 0).map(inert))', '.or_else(|e| Err(e))'),
    ('object-element-silently-dropped', 'crates/romacs-vm/src/mode_line.rs',
     'Form::Object(_) => Form::Vector(vec![]),', 'Form::Object(_) => Form::Sym("nil".into()),'),
    ('column-ignores-tabs', 'crates/romacs-vm/src/display.rs',
     "'\\t' => tab_width - column % tab_width,", "'\\t' => 1,"),
]
results = []
for name, path, old, new in MUTANTS:
    if ONLY and name not in ONLY:
        continue
    f = root / path
    src = f.read_text()
    count = src.count(old)
    if count < 1:
        results.append({'mutant': name, 'error': 'pattern not found'}); continue
    f.write_text(src.replace(old, new))
    try:
        p = subprocess.run(['cargo', 'test', '-p', 'romacs-vm', '--no-fail-fast', '--test', 'tierb_redisplay',
                            '--test', 'tierb_mode_line', '--test', 'mode_line', '--test', 'g3a_end_distance', '-j', '12'],
                           cwd=root, capture_output=True, text=True)
        out = p.stdout + p.stderr
        failed = [l.split()[1] for l in out.splitlines() if l.startswith('test ') and l.endswith('FAILED')]
        panics = [l for l in out.splitlines() if 'panicked at' in l or l.startswith('assertion') ]
        compile_error = 'error[' in out or 'error:' in out and not failed
        results.append({'mutant': name, 'file': path, 'replacements': count, 'exit': p.returncode,
                        'killed_by': failed, 'compile_error': compile_error, 'panics': panics[:6]})
    finally:
        f.write_text(src)
    print(json.dumps(results[-1]), flush=True)
pathlib.Path(sys.argv[1]).write_text(json.dumps(results, indent=1))
