#!/usr/bin/env python3
"""Kill one behavior mutant for each widened primitive, restoring exact bytes.

Run from the repository root. Each mutant must compile and fail its evaluator
assertion; an infrastructure/compile failure is never accepted as a kill.
"""
import os as _portable_os
from pathlib import Path as _PortablePath
from pathlib import Path
import os
import subprocess

ROOT = Path(__file__).resolve().parents[4]
SOURCE = ROOT / 'crates/romacs-vm/src/prims/widen.rs'
LOG = Path(__file__).resolve().parent / 'logs/widen-mutations.log'
MUTANTS = [
    ('symbol-name', 'Ok(cx.string(&name))', 'Ok(cx.string(""))', 'widen_symbol_name_runtime'),
    ('concat', 'Ok(cx.string(&text))', 'Ok(cx.string(""))', 'widen_concat_runtime'),
    ('indirect-function', 'Ok(current)', 'Ok(object)', 'widen_indirect_function_runtime'),
]

def main():
    original = SOURCE.read_bytes()
    env = dict(os.environ)
    env['CARGO_TARGET_DIR'] = str(_PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'lanes/pwide-proofs-target')
    logs = []
    try:
        for name, old, new, test in MUTANTS:
            text = original.decode()
            assert text.count(old) == 1, (name, 'ambiguous mutation')
            SOURCE.write_text(text.replace(old, new))
            cmd = ['nice', '-n', '19', 'cargo', 'test', '--manifest-path', 'crates/romacs-vm/Cargo.toml', '--lib', test, '--', '--nocapture']
            run = subprocess.run(cmd, cwd=ROOT, env=env, text=True, capture_output=True)
            output = run.stdout + run.stderr
            logs.append(f'=== {name}: {cmd!r}; exit={run.returncode} ===\n{output}')
            assert run.returncode == 101 and 'assertion `left == right` failed' in output and '1 failed' in output, (name, 'not a behavioral assertion kill', output)
            print(f'KILLED {name}: evaluator value assertion')
    finally:
        SOURCE.write_bytes(original)
        LOG.write_text('\n'.join(logs))
    cmd = ['nice', '-n', '19', 'cargo', 'test', '--manifest-path', 'crates/romacs-vm/Cargo.toml', '--lib', 'widen_', '--', '--nocapture']
    subprocess.run(cmd, cwd=ROOT, env=env, check=True)
    print('3/3 mutants killed; restored baseline passes')

if __name__ == '__main__':
    main()
