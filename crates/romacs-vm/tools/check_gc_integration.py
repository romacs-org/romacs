#!/usr/bin/env python3
"""Combined collector/language gate using the real public GC API.

Run both release (default) and --debug; the release harness reproduces the VM's
thin-LTO profile so sibling-dispatch code is exercised rather than overflowing.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
from bench import GNU, ROOT

TOOLCHAIN = ''
INSPECT = '''(let ((held (vector (cons 42 nil) (copy-sequence "abc") (vector (cons 7 nil)))))
  (+ (car (aref held 0)) (aref (aref held 1) 1) (car (aref (aref held 2) 0))))'''
STRESS = '''(let ((tag (progn (let ((j 0)) (while (< j 100) (vector j) (setq j (1+ j)))) (vector 99))) (i 0))
 (catch tag (condition-case e
  (progn (while (< i 200000) (vector (cons i nil) (copy-sequence "abc")) (setq i (1+ i)))
         (signal 'error '(42)))
  ((error) (throw tag (car (cdr e)))))))'''


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--debug', action='store_true')
    parser.add_argument('--output', type=Path, default=ROOT / 'evidence/s2-gc-integration.json')
    args = parser.parse_args()
    expected = [subprocess.check_output([GNU, '-Q', '--batch', '--eval',
                f'(prin1 (eval (read {json.dumps(source)}) t))'], text=True).strip()
                for source in (INSPECT, STRESS)]
    env = os.environ.copy()
    env['PATH'] = TOOLCHAIN + ':' + env['PATH']
    scratch = Path.home() / '.hermes/cache/scratch/romacs/vm2'
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='gc-language-gate-', dir=scratch) as directory:
        path = Path(directory)
        # Never inherit another lane's target directory from the shell.
        env['CARGO_TARGET_DIR'] = str(path / 'target')
        manifest = f'''[package]
name = "vm2-language-gc-gate"
version = "0.0.0"
edition = "2024"
[dependencies]
romacs-vm = {{ path = {json.dumps(str(ROOT))} }}
serde_json = "1"
[profile.release]
lto = "thin"
codegen-units = 1
'''
        (path / 'Cargo.toml').write_text(manifest)
        (path / 'src').mkdir()
        (path / 'src/main.rs').write_text((ROOT / 'tools/gc_integration.rs').read_text())
        command = [TOOLCHAIN + '/cargo', 'run', '--offline', *([] if args.debug else ['--release']), '--manifest-path', str(path / 'Cargo.toml'), '--', *expected]
        result = subprocess.run(command, env=env, text=True, capture_output=True)
    passed = result.returncode == 0 and result.stdout.strip() == expected[0]
    report = dict(passed=passed, expected_inspection=expected[0], expected_escape=expected[1],
                  sources=[INSPECT, STRESS], exit=result.returncode, stdout=result.stdout, stderr=result.stderr,
                  scope='combined moving collector; exact accounting and forwarded language/eval roots', debug=args.debug)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(result.stdout + result.stderr)
    print('PASS' if passed else 'RED: collector integration not proven')
    raise SystemExit(0 if passed else 1)

if __name__ == '__main__':
    main()
