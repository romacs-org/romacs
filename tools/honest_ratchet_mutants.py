#!/usr/bin/env python3
"""Prove the honest GNU-suite ratchet can fail, by mutation.

    python3 tools/honest_ratchet_mutants.py [NAME...]

Each mutant removes ONE protection from tools/parity_ratchet.py or
gnu-tests/run.py in a SCRATCH COPY (never this worktree), then runs the ONE
test that protects it. A mutant is KILLED only if that test FAILS (not
errors) and its output carries the test's own assertion message: a mutant
that dies of an import error or another test's assertion proves nothing.
Every edit must apply exactly once (a mutant that does not apply is
indistinguishable from one that survived), and the unmutated copy must pass
the same test first. Exit 0 only if every mutant was killed that way.
"""
from __future__ import annotations

import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
COPY = ['tools/parity_ratchet.py', 'tools/parity_ratchet_test.py', 'tools/honest_ratchet_test.py',
        'gnu-tests/run.py', 'gnu-tests/tests', 'gnu-tests/README.md', 'gnu-tests/member-evidence-policy.json', 'gnu-tests/PIN.json',
        'gnu-tests/EXCLUSIONS.toml', 'gnu-tests/SUSPECT-DIVERGENCES.toml', 'gnu-tests/harness', 'gnu-tests/tools',
        'gnu-tests/ert_report.py', 'parity-reference.toml', 'scripts/gnu_oracle.py']
LINK = ['gnu-tests/test', 'gnu-tests/baseline', 'gnu-tests/runner', 'lisp']
PR, RUN = 'tools/parity_ratchet.py', 'gnu-tests/run.py'
HR = ('tools', 'honest_ratchet_test.HonestRatchet')
HON = ('gnu-tests/tests', 'test_run.Honesty')
E2E = ('gnu-tests/tests', 'test_run.EndToEnd')

# name: (file, old, new, (cwd, test class), test, the test's own assertion message)
MUTANTS = {
    'vacuous-counted-in-ratchet': (
        PR, '    if vacuous:\n        cls, method = \'VACUOUS\'', '    if False:\n        cls, method = \'VACUOUS\'',
        HR, 'test_a_vacuous_pass_cannot_be_counted_as_genuine', 'a vacuous pass was counted as genuine'),
    'vacuous-counted-in-run-py': (
        RUN, '            if vacuous:\n                # GNU\'s guard', '            if False:\n                # GNU\'s guard',
        HON, 'test_a_should_error_satisfied_by_absent_machinery_is_not_a_pass',
        'was counted as a reduced-harness pass'),
    'genuine-loss-is-progress': (
        PR, "    return not unit.startswith('budget:')",
        "    return not (unit.startswith('budget:') or unit.startswith('gnu:'))",
        HR, 'test_a_genuine_regression_still_fails_the_gate', 'a GENUINE floor member was lost and the gate did not fail'),
    'budget-may-grow-in-check': (
        PR, '    status = (EXIT_LOWER if lower or growth else', '    status = (EXIT_LOWER if lower else',
        HR, 'test_the_vacuous_budget_cannot_increase', 'the vacuous budget grew and the gate did not fail'),
    'budget-may-grow-in-record': (
        PR, '        new = [m for m in e[\'gained\'] if m not in moved]', '        new = []',
        HR, 'test_the_vacuous_budget_cannot_increase', 'the vacuous budget grew and'),
    'distro-baseline-accepted': (
        RUN, '    if recorded == expected:\n        return None', '    if True:\n        return None',
        HON, 'test_a_distro_recorded_baseline_is_refused',
        'a baseline recorded by the distro binary was accepted as GNU at the pin'),
    'unloaded-counted-in-ratchet': (
        PR, "'counted': cls == 'GENUINE' and loaded,", "'counted': cls == 'GENUINE',",
        HR, 'test_a_member_whose_file_failed_to_load_is_not_counted',
        'a member whose file failed a top-level form was counted'),
    'unloaded-reported-loaded-in-run-py': (
        RUN, "'file_loaded_clean': not failed and not unreported,", "'file_loaded_clean': True,",
        HON, 'test_a_file_with_a_failed_top_level_form_did_not_load',
        'a file whose top-level require failed was reported as loaded'),
    'progress-is-loss': (
        PR, "    return not unit.startswith('budget:')", '    return True',
        HR, 'test_converting_a_vacuous_pass_into_an_honest_failure_is_progress',
        'converting a vacuous pass into an honest failure read as a floor loss'),
    # End to end: the real adapter and pinned GNU on a fixture (needs
    # ROMACS_GNU_ERT and GNU_EMACS, as the runner's own end-to-end tests do).
    'vacuous-check-off-end-to-end': (
        RUN, '            if vacuous:\n                # GNU\'s guard', '            if False:\n                # GNU\'s guard',
        E2E, 'test_a_vacuous_should_error_pass_is_not_counted_end_to_end',
        'romacs passed a should-error on void-function where GNU caught a real error'),
    'classes-not-rederived': (
        PR, '        if derived != stored:', '        if False:',
        HR, 'test_classes_are_derived_not_hand_written', 'a hand-edited member class was accepted as derived'),
}


def scratch_copy(into: pathlib.Path) -> None:
    for rel in COPY:
        src, dst = ROOT / rel, into / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        if src.is_dir():
            shutil.copytree(src, dst, ignore=shutil.ignore_patterns('__pycache__'))
        else:
            shutil.copy2(src, dst)
    for rel in LINK:
        (into / rel).parent.mkdir(parents=True, exist_ok=True)
        (into / rel).symlink_to(ROOT / rel)


def run_test(root: pathlib.Path, where: tuple[str, str], test: str) -> subprocess.CompletedProcess:
    cwd, cls = where
    return subprocess.run([sys.executable, '-m', 'unittest', '-v', f'{cls}.{test}'], cwd=root / cwd,
                          capture_output=True, text=True, timeout=300)


def main(argv: list[str]) -> int:
    names = argv or list(MUTANTS)
    bad = []
    for name in names:
        rel, old, new, where, test, message = MUTANTS[name]
        with tempfile.TemporaryDirectory(prefix=f'honest-mutant-{name}-') as tmp:
            root = pathlib.Path(tmp)
            scratch_copy(root)
            control = run_test(root, where, test)
            if control.returncode != 0:
                bad.append(f'{name}: the UNMUTATED copy fails {test}:\n{control.stderr[-1500:]}')
                continue
            path = root / rel
            text = path.read_text()
            if text.count(old) != 1:
                bad.append(f'{name}: the edit applies {text.count(old)} times in {rel}, not once')
                continue
            path.write_text(text.replace(old, new))
            if path.read_text() == text:
                bad.append(f'{name}: the edit changed nothing')
                continue
            p = run_test(root, where, test)
            out = p.stdout + p.stderr
            killed = p.returncode != 0 and f'FAIL: {test}' in out and f'ERROR: {test}' not in out and message in out
            line = next((l for l in out.splitlines() if message in l), '')
            print(f"{'KILLED  ' if killed else 'SURVIVED'} {name}: {test} -- {line.strip()[:220]}")
            if not killed:
                bad.append(f'{name}: not killed at its own assertion ({test}, expected {message!r}):\n{out[-2000:]}')
    if bad:
        print('\n'.join(bad))
        return 1
    print(f'all {len(names)} mutants killed, each at its own test\'s assertion message')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
