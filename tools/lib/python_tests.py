#!/usr/bin/env python3
"""Run the repository's Python unit tests: every tracked test file that no
other runner owns, each in a process of its own with a private HOME and
TMPDIR, in parallel.

A test file is a tracked `test_*.py', `test-*.py' or `*_test.py' that uses
unittest, outside the GNU and Lisp trees. One a Rust test or the Makefile
already runs is listed in ELSEWHERE with that runner, which must name it.
Every other one runs here, so a new test file cannot go unrun. A file
passes only when at least one test body ran (not skipped) and all passed.

usage: python_tests.py [--root DIR] [--jobs N] [FILE ...]
"""
import argparse
import concurrent.futures
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time

NAME = re.compile(r'(^|/)(test_[^/]*|test-[^/]*|[^/]*_test)\.py$')
OUTSIDE = ('gnu-lisp/', 'gnu-tests/test/', 'test/')
# Test files another runner owns: path -> the runner that names it.
ELSEWHERE = {
    **{f'gnu-tests/tests/{name}.py': 'gnu-tests/runner/tests/python_contract.rs'
       for name in ('test_run', 'test_ert_report', 'test_real_romacs', 'test_ert_adapter',
                    'test_scoreboard', 'test_vacuity_classifier', 'test_evidence_local',
                    'test_audit_manifest', 'test_gate_vacuity')},
    'crates/romacs-vm-gui/harness/test_fileio_live.py': 'crates/romacs-vm-gui/tests/fileio_observer.rs',
    'crates/romacs-vm-gui/harness/test_find_file_live.py': 'crates/romacs-vm-gui/tests/find_file.rs',
    'crates/romacs-vm-gui/harness/test_scaling_live.py': 'crates/romacs-vm-gui/tests/scaling.rs',
    'tools/tests/parity_ratchet_test.py': 'gnu-tests/runner/tests/parity_ratchet.rs',
    'tools/tests/honest_ratchet_test.py': 'tools/tests/parity_ratchet_test.py',
    # Needs a built host, which only Cargo can promise.
    'tools/tests/test_elisp_runner.py': 'crates/romacs-vm/tests/elisp_runner.rs',
}
RAN = re.compile(r'^Ran (\d+) tests? in ', re.M)
SKIPPED = re.compile(r'^(?:OK|FAILED) \(.*?\bskipped=(\d+)', re.M)


def test_files(root):
    """The tracked unittest files under ROOT, as root-relative paths."""
    listed = subprocess.check_output(['git', '-C', str(root), 'ls-files', '-z'], text=True).split('\0')
    return sorted(path for path in listed
                  if NAME.search(path) and not path.startswith(OUTSIDE)
                  and 'unittest' in (root / path).read_text(errors='replace'))


def unowned(root, files):
    """FILES no other runner owns; an ELSEWHERE entry whose file or runner
    is gone, or whose runner does not name it, is an error."""
    for path, runner in ELSEWHERE.items():
        runner_text = (root / runner).read_text() if (root / runner).is_file() else ''
        if path not in files or Path(path).stem not in runner_text:
            raise ValueError(f'{path}: listed as run by {runner}, which does not run it')
    return [path for path in files if path not in ELSEWHERE]


def command(path):
    """Run PATH's tests: as a module where its name is one, else as a script."""
    name = Path(path).stem
    return ([sys.executable, '-B', '-m', 'unittest', name] if name.isidentifier()
            else [sys.executable, '-B', Path(path).name])


def oracle_root():
    """The pinned GNU oracle's cache, resolved against the caller's HOME before a
    test's private HOME hides it (as tools/lib/gnu_oracle.py's callers find it)."""
    return os.environ.get('GNU_ORACLE_ROOT') or str(
        Path(os.environ.get('XDG_CACHE_HOME') or Path.home() / '.cache') / 'gnu-emacs-pin')


def run(root, path, timeout=300):
    """(PATH, ok, seconds, output tail) for one test file."""
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix='pytest-') as private:
        home, tmp = Path(private, 'home'), Path(private, 'tmp')
        home.mkdir(), tmp.mkdir()
        env = {**os.environ, 'GNU_ORACLE_ROOT': oracle_root(), 'HOME': str(home), 'TMPDIR': str(tmp),
               'PYTHONDONTWRITEBYTECODE': '1'}
        try:
            done = subprocess.run(command(path), cwd=(root / path).parent, env=env, stdin=subprocess.DEVNULL,
                                  capture_output=True, text=True, timeout=timeout)
            output, code = done.stdout + done.stderr, done.returncode
        except subprocess.TimeoutExpired:
            output, code = f'timed out after {timeout}s', -1
        finally:
            subprocess.run(['chmod', '-R', 'u+w', private], check=False)
    executed = sum(int(n) for n in RAN.findall(output)) - sum(int(n) for n in SKIPPED.findall(output))
    return path, code == 0 and executed > 0, time.monotonic() - started, '\n'.join(output.splitlines()[-15:])


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--root', default='.')
    parser.add_argument('--jobs', type=int, default=8)
    parser.add_argument('files', nargs='*', help='only these (root-relative) test files')
    args = parser.parse_args()
    root = Path(args.root).resolve()
    try:
        selected = args.files or unowned(root, test_files(root))
    except ValueError as error:
        print(f'python-tests: {error}', file=sys.stderr)
        return 1
    started = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        results = list(pool.map(lambda path: run(root, path), selected))
    failed = [r for r in results if not r[1]]
    for path, _, seconds, tail in failed:
        print(f'python-tests: FAIL {path} ({seconds:.1f}s)\n{tail}', file=sys.stderr)
    slowest = max(results, key=lambda r: r[2], default=None)
    print(f'python-tests: {len(results) - len(failed)}/{len(results)} files passed in '
          f'{time.monotonic() - started:.1f}s'
          + (f' (slowest {slowest[0]} {slowest[2]:.1f}s)' if slowest else ''))
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
