#!/usr/bin/env python3
"""Mutation proof for the loadup lane's guards.

    mutants.py ROOT

Each mutant edits one source file the way a plausible regression would, asserts
the edit applied (a mutant that fails to apply is indistinguishable from a
surviving one, rule 22), runs the named test, and requires it to FAIL with a
panic at the named assertion line. The source is restored after each mutant.
"""
import subprocess, sys, re
from pathlib import Path

root = Path(sys.argv[1]).resolve()
SRC = 'crates/romacs-vm-gui/src/'
# (name, file, old, new, test filter, file:line-pattern of the target assertion)
MUTANTS = [
    ('site failure ends the drive (GNU dump regime at file level)', 'loadup.rs',
     "     (if (memq (car romacs-loadup--error) '(file-error file-missing))\n         (signal (car romacs-loadup--error) (cdr romacs-loadup--error)))",
     "     (signal (car romacs-loadup--error) (cdr romacs-loadup--error))",
     'loadup::tests::failed_form_ends_its_file_but_not_the_drive', r'form_failures'),
    ('wrapper swallows file-error (loadup handler never runs)', 'loadup.rs',
     "(memq (car romacs-loadup--error) '(file-error file-missing))",
     "(memq (car romacs-loadup--error) '(no-such-condition))",
     'loadup::tests::failed_form_ends_its_file_but_not_the_drive', r'assert_eq!\(vars'),
    ('takeover by fboundp, not by identity (label upgrade)', 'startup.rs',
     "\"(and (fboundp '{command}) (not (eq (symbol-function '{command}) (cdr (assq '{command} romacs-gui--before-loadup)))))\"",
     "\"(fboundp '{command})\"",
     'startup::tests::gnu_commands_take_over_only_where_loadup_defined_them', r'assert_eq!\(keys'),
    ('drive stops at the first loadup form that signals (GNU dump regime)', 'loadup.rs',
     "            form_failures.push(FormFailure { form: k, line, source: source[start..end].to_owned(), error });\n        }",
     "            form_failures.push(FormFailure { form: k, line, source: source[start..end].to_owned(), error });\n            break;\n        }",
     'startup::tests::loadup_launch_runs_all_of_loadup_and_keeps_a_cursor', r'loadup stopped inside the old chain'),
    ('host helpers use window-child after GNU window.el replaced it', 'startup.el',
     "    (let ((child (romacs-gui--window-child w)) (leaves nil))",
     "    (let ((child (window-child w)) (leaves nil))",
     'startup::tests::loadup_launch_runs_all_of_loadup_and_keeps_a_cursor', r'assert_eq!\(live'),
    ('C-x C-e stand-in answers under GNU\'s name', 'startup.rs',
     '    ("[24 5]", "eval-last-sexp", "romacs-gui-eval-last-sexp"),',
     '    ("[24 5]", "eval-last-sexp", "eval-last-sexp"),',
     'startup::tests::loadup_launch_runs_all_of_loadup_and_keeps_a_cursor', r'assert_eq!\(gnu, !command'),
    ('no stand-in mode line after loadup (no cursor)', 'startup.rs',
     '    let mode_line = if started.gnu_mode_line { "nil" } else { "t" };',
     '    let mode_line = "nil";',
     'startup::tests::loadup_launch_runs_all_of_loadup_and_keeps_a_cursor', r'the default window has no mode line'),
    ('scanner reads code after a ; comment start', 'loadup.rs',
     "                while i < bytes.len() && bytes[i] != b'\\n' { i += 1; }",
     "                i += 1;",
     'loadup::tests::load_sites_skip_strings_comments_and_characters', r'assert_eq!\(spans.len\(\)|assert_eq!\(names'),
    ('bundled Lisp not appended after loadup', 'startup.rs',
     "    if !bundled.is_empty() {\n        vm.eval_source",
     "    if false {\n        vm.eval_source",
     'startup::tests::loadup_launch_runs_all_of_loadup_and_keeps_a_cursor', r'assert_eq!\(bundled'),
    ('lisp dir not canonicalized before finding lisp-bundled', 'startup.rs',
     "    let dir = &dir.canonicalize().map_err(|e| format!(\"GNU lisp directory {}: {e}\", dir.display()))?;",
     "    let dir = &dir.to_path_buf();",
     'startup::tests::loadup_finds_bundled_lisp_through_a_symlinked_lisp_dir', r'assert_eq!\(found'),
    # vm2/loadpath: GNU's subdirs.el step (loadup.rs apply_subdirs) and its order.
    ('subdirs.el step skipped (load-path stops at loadup.el\'s 7)', 'loadup.rs',
     "\"(fboundp 'normal-top-level)\")? == romacs_vm::Fixed::NIL {",
     "\"(fboundp 'normal-top-level)\")? != romacs_vm::Fixed::NIL {",
     'loadup::tests::loadup_load_path_reaches_gnu_subdirectories_in_gnu_order', r'Org is not findable'),
    ('subdirs.el step skipped: bundled gnus-dbus found in place of GNU\'s', 'loadup.rs',
     "\"(fboundp 'normal-top-level)\")? == romacs_vm::Fixed::NIL {",
     "\"(fboundp 'normal-top-level)\")? != romacs_vm::Fixed::NIL {",
     'startup::tests::loadup_load_path_finds_every_gnu_file_before_bundled_lisp', r'gnus-dbus is not GNU'),
    ('bundled Lisp put before GNU\'s load-path', 'startup.rs',
     '"(setq load-path (append load-path (list {})))"',
     '"(setq load-path (append (list {}) load-path))"',
     'startup::tests::loadup_load_path_finds_every_gnu_file_before_bundled_lisp', r'gnus-dbus is not GNU'),
    ('obsolete/ not moved last (update-subdirs order)', 'loadup.rs',
     '        let append = file == "obsolete";',
     '        let append = false;',
     'loadup::tests::loadup_load_path_reaches_gnu_subdirectories_in_gnu_order', r'assert_eq!\(path, expected'),
    ('obsolete/ not moved last: script text differs', 'loadup.rs',
     '        let append = file == "obsolete";',
     '        let append = false;',
     'loadup::tests::update_subdirs_writes_what_gnus_script_writes', r'assert_eq!\(text\(&lisp\)\.map'),
    ('term/ not excluded (update-subdirs)', 'loadup.rs',
     '[&b"term"[..], b"RCS", b"CVS", b"Old"]', '[&b"RCS"[..], b"CVS", b"Old"]',
     'loadup::tests::loadup_load_path_reaches_gnu_subdirectories_in_gnu_order', r'assert_eq!\(absent'),
    ('cedet/ and leim/ get their own subdirs.el (SUBDIRS_SUBDIRS ignored)', 'loadup.rs',
     '    if rel.starts_with("cedet") || rel.starts_with("leim") { return SubdirsEl::None; }\n', '',
     'loadup::tests::loadup_load_path_reaches_gnu_subdirectories_in_gnu_order', r'assert_eq!\(absent'),
    ('entries appended at the end, not spliced after their parent', 'loadup.rs',
     'while at + 1 < load_path.len() && load_path[at] != thisdir { at += 1; }',
     'while at + 1 < load_path.len() { at += 1; }',
     'loadup::tests::loadup_load_path_reaches_gnu_subdirectories_in_gnu_order', r'assert_eq!\(path, expected'),
    ('entries a subdirs.el added are not themselves visited', 'loadup.rs',
     '    let mut i = 0;\n    while i < load_path.len() {',
     '    let mut i = 0;\n    let n = load_path.len();\n    while i < n {',
     'loadup::tests::apply_subdirs_splices_as_gnus_startup_loop_does', r'assert_eq!\(path, gnu\)'),
    ('symlinked directory decided by its own spelling, not its target', 'loadup.rs',
     '    let Ok(real) = dir.canonicalize() else { return SubdirsEl::None };\n', '    let real = dir.to_path_buf();\n',
     'loadup::tests::apply_subdirs_follows_gnus_build_through_symlinks', r'assert_eq!\(path, gnu'),
    ('names outside the reproduced domain (~user, escapes, quotes, bytes, spaces) applied', 'loadup.rs',
     "    !name.is_empty() && name[0] != b'~' && name.iter().all(|&b| b.is_ascii_alphanumeric() || b\"-_.+,@=~\".contains(&b))",
     "    let _ = name; true",
     'loadup::tests::apply_subdirs_applies_nothing_outside_its_domain_and_says_so', r'a directory outside the domain reached'),
]

env = dict(__import__('os').environ)
fail = 0
for name, file, old, new, test, target in MUTANTS:
    path = root / SRC / file
    original = path.read_text()
    if original.count(old) != 1:
        print(f'NOT-APPLIED {name}: pattern count {original.count(old)}'); fail += 1; continue
    path.write_text(original.replace(old, new))
    try:
        assert path.read_text() != original
        p = subprocess.run(['cargo', 'test', '--locked', '-p', 'romacs-vm-gui', '--bin', 'romacs-vm-gui', test, '--', '--exact'],
                           cwd=root, capture_output=True, text=True, env=env)
        out = p.stdout + p.stderr
        compiled = 'test result' in out or 'panicked' in out
        failed = 'FAILED' in out or 'test result: FAILED' in out
        # the panic's source line must be the target assertion
        # Read the MUTATED source: a mutant that adds or removes lines moves them.
        lines = re.findall(r'panicked at (crates/romacs-vm-gui/src/[^:]+):(\d+)', out)
        at_target = False
        for f, n in lines:
            text = (root / f).read_text().splitlines()[int(n) - 1]
            if re.search(target, text): at_target = True
        status = 'KILLED' if compiled and failed and at_target else ('KILLED-ELSEWHERE' if failed else ('NO-COMPILE' if not compiled else 'SURVIVED'))
        fail += status != 'KILLED'
        first = next((l for l in out.splitlines() if 'panicked at' in l), '')
        print(f'{status:16s} {name}  [{test}]  {first.strip()[:160]}')
    finally:
        path.write_text(original)
        assert path.read_text() == original
print('RESULT', 'all killed at their target assertion' if not fail else f'{fail} not killed at target')
sys.exit(1 if fail else 0)
