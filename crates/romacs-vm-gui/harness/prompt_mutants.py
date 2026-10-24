#!/usr/bin/env python3
"""Single-site mutants for the host-keyboard prompt (gui/prompt lane).

Each mutant edits one site, ASSERTS the edit applied (the old text existed
exactly once and the file changed), runs the tests that protect it, records
pass/fail and the first failing assertion, and restores the file byte-exact.
A mutant is KILLED when its test command fails.
"""
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[3]
LR = str(ROOT / 'scripts/lane-run')
VM = 'crates/romacs-vm/src/'
HOST_TEST = ['cargo', 'test', '-p', 'romacs-vm', '--test', 'host_input']
LIB_TEST = ['cargo', 'test', '-p', 'romacs-vm', '--lib', 'host_input']
CURSOR_TEST = ['cargo', 'test', '-p', 'romacs-layout2', '--test', 'cursor']

MUTANTS = [
    ('M1 gate reverted: no host means end-of-file', VM + 'prims/minibuf.el',
     '(unless (or executing-kbd-macro (romacs--minibuf-host-input-p))', '(unless executing-kbd-macro', HOST_TEST),
    ('M2 read-event never asks the host', VM + 'prims/minibuf.el',
     '         (romacs--read-host-event))))', '         nil)))', HOST_TEST),
    ('M3 host read is a plain primitive (no reentrant stack/GC discipline)', VM + 'threaded.rs',
     'Primitive::Eval | Primitive::AutoloadDoLoad | Primitive::ReadHostEvent) {\n            return reentrant_load(',
     'Primitive::Eval | Primitive::AutoloadDoLoad) {\n            return reentrant_load(', HOST_TEST),
    ('M4 pending input ignored (always redisplay)', VM + 'host_input.rs',
     'if !host.input_pending() {', 'if { let _ = host.input_pending(); true } {', HOST_TEST),
    ('M5 no redisplay before the wait', VM + 'host_input.rs',
     'if !host.input_pending() {', 'if false && !host.input_pending() {', HOST_TEST),
    ('M6 closed frame returns nil instead of signalling', VM + 'host_input.rs',
     '                Err("quit: frame closed while reading input".into())', '                { let _ = &self.propagated_escape; self.propagated_escape = None; Ok(Value::NIL) }', HOST_TEST),
    ('M7 resize not applied', VM + 'host_input.rs',
     '                    self.set_frame_geometry(width, height, line, column)?;', '                    let _ = (width, height, line, column);', HOST_TEST),
    ('M8 host not restored after a read', VM + 'host_input.rs',
     '        self.host_input = Some(host);\n        match outcome?', '        drop(host);\n        match outcome?', HOST_TEST),
    ('M9 selected id is the first window, not the selected one', VM + 'host_input.rs',
     'let ends = host.redisplay(HostFrame { frame, selected })?;',
     'let selected = frame.windows[0].snapshot.id; let ends = host.redisplay(HostFrame { frame, selected })?;', HOST_TEST),
    ('M10 layout ends from the wait not committed', VM + 'host_input.rs',
     '                self.display_apply_layout(&ends)?;', '                let _ = ends;', HOST_TEST),
    ('M11 C-g returns instead of signalling (abort exits normally)', VM + 'prims/minibuf.el',
     "      (throw 'exit (lambda () (signal 'minibuffer-quit nil))))))", "      (throw 'exit nil))))", HOST_TEST),
    ('M12 Closed signals error, not quit', VM + 'host_input.rs',
     'let quit = self.intern("quit");', 'let quit = self.intern("error");', HOST_TEST),
    ('M13 active minibuffer gets no cursor (pre-fix layout)', 'crates/romacs-layout2/src/lib.rs',
     'cursor.filter(|&(id, _)| placed.snapshot.mode_line.is_some() || id == placed.snapshot.id)',
     'cursor.filter(|_| placed.snapshot.mode_line.is_some())', CURSOR_TEST),
    ('M17 a closed frame is not sticky (a caught quit waits again)', 'crates/romacs-vm-gui/src/prompt.rs',
     '    if shared.closed {\n        return Next::Closed;\n    }\n', '', ['cargo', 'test', '-p', 'romacs-vm-gui', '--bin', 'romacs-vm-gui', 'prompt::']),
    ('M14 funcall/apply path treats the read as an ordinary value primitive', VM + 'threaded.rs',
     'Primitive::Eval | Primitive::AutoloadDoLoad | Primitive::ReadHostEvent => {\n                t2_invoke_full(',
     'Primitive::Eval | Primitive::AutoloadDoLoad => {\n                t2_invoke_full(', LIB_TEST),
    ('M15 indirect call path does not route the read to engine control', VM + 'threaded.rs',
     '                || target == Value::reference(Primitive::ReadHostEvent as usize, 6)\n', '', HOST_TEST),
    ('M16 read caches as an inlinable ordinary primitive', VM + 'threaded.rs',
     '                    | Primitive::AutoloadDoLoad\n                    | Primitive::ReadHostEvent\n            ) {',
     '                    | Primitive::AutoloadDoLoad\n            ) {', HOST_TEST),
]


def main():
    only = set(sys.argv[1:])
    results = []
    for name, path, old, new, test in MUTANTS:
        if only and name.split()[0] not in only:
            continue
        file = ROOT / path
        original = file.read_bytes()
        text = original.decode()
        assert text.count(old) == 1, (name, 'site not found exactly once', text.count(old))
        file.write_text(text.replace(old, new))
        applied = file.read_bytes() != original
        assert applied, (name, 'mutant did not change the file')
        try:
            run = subprocess.run([LR, 'gpr', '-C', str(ROOT), '--', *test], capture_output=True, text=True, timeout=900)
            out = run.stdout + run.stderr
            failed = [l for l in out.splitlines() if l.startswith('test ') and l.endswith('FAILED')]
            panics = [l.strip() for l in out.splitlines() if 'panicked at' in l or l.startswith('  left') or l.startswith(' right')][:6]
            # A kill is a behavioural failure: a failing test, a panic at an
            # assertion, or the test process dying (stack overflow, signal).
            # A mutant that does not compile is NOT a kill.
            compile_error = 'error[E' in out or 'could not compile' in out
            crashed = 'process didn\'t exit successfully' in out and 'signal:' in out
            killed = run.returncode != 0 and not compile_error and bool(failed or panics or crashed)
            results.append({'mutant': name, 'file': path, 'applied': applied, 'exit': run.returncode,
                            'killed': killed, 'compile_error': compile_error, 'crashed': crashed,
                            'failed_tests': failed, 'first_failures': panics,
                            'tail': [l for l in out.splitlines() if l.strip()][-4:]})
        finally:
            file.write_bytes(original)
        assert file.read_bytes() == original
        print(json.dumps(results[-1], ensure_ascii=False), flush=True)
    print(json.dumps({'total': len(results), 'killed': sum(r['killed'] for r in results)}))


if __name__ == '__main__':
    main()
