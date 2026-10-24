#!/usr/bin/env python3
"""Prove the parity ratchet can fail, in both directions, by mutation.

    python3 tools/parity_ratchet_mutants.py --gnu-ert BIN --census-bin BIN [NAME...]

PRODUCT mutants change what is measured (a Lisp definition, a GNU file the
census loads) in a SCRATCH COPY of the tree -- never in this worktree's lisp/
-- and run the real checker on that copy. CHECKER mutants change
tools/parity_ratchet.py in the scratch copy and require its own tests
(tools/parity_ratchet_test.py) to fail. Every mutant asserts that its edit
applied (the old text occurs exactly once and the bytes changed): a mutant
that fails to apply is indistinguishable from one that survived.

Exit 0 only if every mutant got its expected verdict and the unmutated copy
passes every unit it measured before and after.
"""
from __future__ import annotations
import os as _portable_os
from pathlib import Path as _PortablePath

import argparse
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
COPY = ['lisp', 'gnu-tests/test', 'gnu-tests/baseline', 'gnu-tests/member-evidence-policy.json', 'parity-reference.toml',
        'scripts/gnu_oracle.py', 'gnu-tests/harness', 'gnu-tests/tools', 'gnu-tests/run.py',
        'gnu-tests/PIN.json', 'gnu-tests/EXCLUSIONS.toml', 'gnu-tests/SUSPECT-DIVERGENCES.toml', 'tools',
        'scripts/lisp-census.py', 'scripts/lisp-census-oracle.el', 'parity-ratchet.json', 'DIVERGENCES.md',
        'crates/romacs-face/GNU-DIVERGENCES.md', 'crates/romacs-regex-engine/GNU-DIVERGENCES.md',
        'crates/romacs-vm/GNU-DIVERGENCES.md', 'gnu-tests/runner/tests/parity_ratchet.rs',
        'crates/romacs-vm/tests/parity_ratchet.rs', 'scripts/gate', 'scripts/gate_test.py']

SUBR_FILES = 'gnu-files:lisp/subr-tests.el'


def first_cold_loaded(root: pathlib.Path) -> str:
    return json.loads((root / 'parity-ratchet.json').read_text())['census']['cold']['loaded'][0]


def first_cold_failing_gnu(root: pathlib.Path) -> str:
    data = json.loads((root / 'parity-ratchet.json').read_text())
    loaded = set(data['census']['cold']['loaded'])
    out = subprocess.run(['git', '-C', str(_PortablePath(_portable_os.environ.get('ROMACS_GNU_SOURCE', _PortablePath(_portable_os.environ.get('GNU_ORACLE_ROOT', _PortablePath(_portable_os.environ.get('XDG_CACHE_HOME', _PortablePath.home() / '.cache')) / 'gnu-emacs-pin')) / 'src')) / ''), 'ls-tree', '-r', '--name-only',
                          '45b0f7699df36b3849edc3bdef37d4cb9e75b9ea', '--', 'lisp/emacs-lisp'],
                         capture_output=True, text=True, check=True).stdout.split()
    return next(p for p in sorted(out) if p.endswith('.el') and p not in loaded and (root / p).is_file())


# name: (kind, file (callable on root or str), old, new, how to run, expected exit, required output)
def mutants(root: pathlib.Path) -> dict:
    loaded = first_cold_loaded(root)
    failing = first_cold_failing_gnu(root)
    return {
        'product-drops-a-gnu-pass': (
            'lisp/subr.el', '  (car (car (car x))))', '  (car (car x)))',
            ['check', '--unit', SUBR_FILES], 1, ['FAIL gnu:lisp/subr-tests.el', '- subr-test-caaar: now']),
        'product-raises-a-gnu-count': (
            'lisp/subr.el', '\n;;; Code:\n',
            "\n;;; Code:\n(defun char-uppercase-p (c) (and (memq c (list ?R ?S ?\u03a9 ?\u03a8)) t))\n",
            ['check', '--unit', SUBR_FILES], 3,
            ['HIGHER gnu:lisp/subr-tests.el', 'test-char-uppercase-p', 'FAIL-WITH-INSTRUCTION',
             'python3 tools/parity_ratchet.py record']),
        'product-census-file-stops-loading': (
            loaded, None, '(error "parity-ratchet mutant")\n',
            ['check', '--unit', 'census-cold'], 1, [f'FAIL census-cold', f'- {loaded}: now']),
        'product-census-file-starts-loading': (
            failing, 'WHOLE', ';;; -*- lexical-binding: t -*-\nnil\n',
            ['check', '--unit', 'census-cold'], 3, ['HIGHER census-cold', failing, 'FAIL-WITH-INSTRUCTION']),
        'adapter-runs-nothing': (
            None, None, None, ['check', '--unit', SUBR_FILES, '--gnu-ert', '/bin/true'], 4,
            ['NOT-MEASURED', 'zero tests executed']),
        'census-binary-reports-nothing': (
            None, None, None, ['check', '--unit', 'census-cold', '--census-bin', '/bin/true'], 4,
            ['NOT-MEASURED']),
        # Checker mutants: tools/parity_ratchet.py loses a protection; its own
        # tests must fail.
        'checker-ignores-losses': (
            'tools/parity_ratchet.py', "    lower = [e for e in entries if floor_unit(e['unit']) and e['lost']]",
            "    lower = []",
            'TESTS', None, ['test_a_lost_pass_is_fail_and_names_it']),
        'checker-passes-gains-silently': (
            'tools/parity_ratchet.py',
            "    higher = [e for e in entries if floor_unit(e['unit']) and not e['lost'] and e['gained']]",
            "    higher = []",
            'TESTS', None, ['test_a_gain_is_fail_with_instruction']),
        'checker-accepts-zero-tests': (
            'tools/parity_ratchet.py', '    if reported == 0:\n        raise Refusal(EXIT_NOT_MEASURED, f\'romacs reported',
            '    if False:\n        raise Refusal(EXIT_NOT_MEASURED, f\'romacs reported',
            'TESTS', None, ['test_an_adapter_that_runs_nothing_is_not_measured']),
        'checker-refusal-exits-zero': (
            'tools/parity_ratchet.py', "        return e.status\n    return EXIT_INVALID",
            "        return 0\n    return EXIT_INVALID", 'TESTS', None,
            ['test_refusal_exits_nonzero_and_never_prints_pass']),
        'checker-ignores-denominator': (
            'tools/parity_ratchet.py', '    problems = denominators(data, m)\n', '    problems = []\n',
            'TESTS', None, ['test_a_moved_denominator_is_not_measured']),
        'verify-ignores-silent-lowering': (
            'tools/parity_ratchet.py', "        hidden = [m for m in e['lost'] if m not in covered[e['unit']]]",
            "        hidden = []", 'TESTS', None, ['test_silent_lowering_is_refused']),
        'record-compares-only-new-units': (
            'tools/parity_ratchet.py', "    result = compare(floors(old), {**{u: [] for u in floors(old)}, **floors(data)}, verdicts)",
            "    result = compare(floors(old), floors(data), verdicts)", 'TESTS', None,
            ['test_losing_a_whole_file_floor_needs_lower']),
        'report-accepts-zero-reported': (
            'tools/parity_ratchet.py', "    if not (reported > 0 and denom > 0 and units):", "    if False:",
            'TESTS', None, ['test_nothing_reported_is_never_pass']),
        'record-accepts-one-run': (
            'tools/parity_ratchet.py', "            if args.repeat < 2:", "            if args.repeat < 1:",
            'TESTS', None, ['test_one_run_is_refused']),
        'contamination-ignores-ignored-lisp': (
            'tools/parity_ratchet.py', "    return (parts[:1] == ('scripts',) and '__pycache__' in parts",
            "    return ('__pycache__' in parts", 'TESTS', None, ['test_only_python_bytecode_is_harmless']),
        'verify-old-records-authorize-new-losses': (
            'tools/parity_ratchet.py', "        added = after[len(before):]\n", "        added = after\n",
            'TESTS', None, ['test_an_old_lowering_record_does_not_authorize_a_new_loss']),
        'verify-only-against-merge-base': (
            'tools/parity_ratchet.py',
            "                 git(root, 'rev-list', '--parents', '--topo-order', '--reverse', f'{base}..HEAD').splitlines()]",
            "                 []]", 'TESTS', None, ['test_a_lane_floor_dropped_before_a_forward_merge_is_still_refused']),
        'verify-merge-demands-a-prefix': (
            'tools/parity_ratchet.py', "merge=len(ps) > 1)", "merge=False)", 'TESTS', None,
            ['test_independent_recorded_lowerings_merge_in_either_order']),
        'verify-merge-forgets-a-parents-record': (
            'tools/parity_ratchet.py',
            "            else:\n                problems.append(f'{label}: earlier lowering records were changed or deleted')\n                break\n",
            "            else:\n                break\n", 'TESTS', None,
            ['test_a_merge_that_deletes_a_parents_lowering_record_is_refused']),
        'verify-treats-a-pre-ratchet-parent-as-the-base': (
            'tools/parity_ratchet.py', "            old = at(parent)\n", "            old = at(parent) or at(base)\n",
            'TESTS', None, ['test_pre_ratchet_lane_commits_are_not_deletions']),
        'verify-compares-a-merge-with-its-first-parent-only': (
            'tools/parity_ratchet.py', "        for parent in ps:\n", "        for parent in ps[:1]:\n",
            'TESTS', None, ['test_a_lowering_inside_the_forward_merge_is_still_refused']),
        'verify-walks-only-the-ancestry-path': (
            'tools/parity_ratchet.py', "'rev-list', '--parents', '--topo-order', '--reverse', f'{base}..HEAD'",
            "'rev-list', '--parents', '--ancestry-path', '--first-parent', '--reverse', f'{base}..HEAD'",
            'TESTS', None, ['test_a_lane_floor_dropped_before_a_forward_merge_is_still_refused']),
        'verify-walks-only-the-first-parent-chain': (
            'tools/parity_ratchet.py', "'rev-list', '--parents', '--topo-order', '--reverse', f'{base}..HEAD'",
            "'rev-list', '--parents', '--first-parent', '--reverse', f'{base}..HEAD'",
            'TESTS', None, ['test_a_floor_raised_and_dropped_on_a_merged_side_branch_is_refused']),
        'wiring-ignores-the-pin': (
            'tools/parity_ratchet.py', "        if actual != pinned:", "        if False:",
            'TESTS', None, ['test_a_dropped_shard_is_refused', 'test_a_compiled_out_test_file_is_refused',
                            'test_a_shard_without_its_test_attribute_is_refused']),
        # The confirmation (REJECT of 13b4d4aa): two runs that each fail must
        # not merge into a pass; every forgiveness is visible, counted and
        # escalated; a confirmation must be comparable with the first run.
        'confirmation-checks-only-the-lost-members': (
            'tools/parity_ratchet.py',
            "        missed = sorted(set(fl.get(unit, [])) - set(second.get(unit, [])))",
            "        missed = sorted(set(members) - set(second.get(unit, [])))", 'TESTS', None,
            ['test_two_runs_that_each_lose_a_different_member_are_not_a_pass',
             'test_a_different_member_lost_in_the_confirmation_clears_nothing']),
        'confirmation-forgives-everything': (
            'tools/parity_ratchet.py', "        if missed:\n            reproduced[unit] = missed",
            "        if False:\n            reproduced[unit] = missed", 'TESTS', None,
            ['test_a_reproduced_loss_stays_lost', 'test_a_member_absent_in_both_runs_fails']),
        'confirmation-adds-gains': (
            'tools/parity_ratchet.py', "            cleared[unit] = sorted(set(members) & set(second.get(unit, [])))",
            "            cleared[unit] = sorted(set(second.get(unit, [])))", 'TESTS', None,
            ['test_confirmation_never_adds_gains']),
        'report-hides-unconfirmed': (
            'tools/parity_ratchet.py', "        print(f'{PREFIX} UNCONFIRMED {unit}: {line}')", "        pass",
            'TESTS', None, ['test_a_cleared_loss_passes_visibly_counted_and_noticed']),
        'pass-line-hides-the-count': (
            'tools/parity_ratchet.py', "          f'unconfirmed {sum(", "          f'unconfirmed {0 and sum(",
            'TESTS', None, ['test_a_cleared_loss_passes_visibly_counted_and_noticed']),
        'unconfirmed-claims-a-cause': (
            'tools/parity_ratchet.py', "+ f'. Cause NOT established (host disturbance and a product race look alike here); '",
            "+ f'. The first run was disturbed; not a regression; '", 'TESTS', None,
            ['test_the_unconfirmed_line_claims_no_cause']),
        'ledger-not-written': (
            'tools/parity_ratchet.py', "        f.write(''.join(json.dumps(r, sort_keys=True) + '\\n' for r in rows))",
            "        f.write('')", 'TESTS', None,
            ['test_a_cleared_loss_passes_visibly_counted_and_noticed',
             'test_the_same_member_forgiven_again_escalates_to_fail']),
        'ledger-follows-private-home': (
            'tools/parity_ratchet.py', "    return pathlib.Path(pwd.getpwuid(os.getuid()).pw_dir) /",
            "    return pathlib.Path(os.environ['HOME']) /", 'TESTS', None,
            ['test_the_ledger_ignores_a_private_home']),
        'escalation-disabled': (
            'tools/parity_ratchet.py', "                outcome = 'escalated' if prior >= UNCONFIRMED_ALLOWED else 'forgiven'",
            "                outcome = 'forgiven'", 'TESTS', None,
            ['test_the_same_member_forgiven_again_escalates_to_fail']),
        'escalation-ignores-the-window': (
            'tools/parity_ratchet.py', "\n                            and r['t'] >= now - UNCONFIRMED_WINDOW_DAYS * 86400)", ")",
            'TESTS', None, ['test_a_forgiveness_outside_the_window_does_not_count']),
        'escalation-counts-any-member': (
            'tools/parity_ratchet.py', " and r.get('member') == member\n", "\n",
            'TESTS', None, ['test_another_members_forgiveness_does_not_count']),
        'gate-notice-not-written': (
            'tools/parity_ratchet.py', "    if target and lines:", "    if False:", 'TESTS', None,
            ['test_a_cleared_loss_passes_visibly_counted_and_noticed',
             'test_the_same_member_forgiven_again_escalates_to_fail']),
        'unrecordable-forgiveness-granted': (
            'tools/parity_ratchet.py',
            "        conf['invalid'] = f'the forgiveness could not be recorded ({e}); an uncounted loss is never forgiven'\n"
            "        return m",
            "        conf['invalid'] = f'the forgiveness could not be recorded ({e}); an uncounted loss is never forgiven'\n"
            "        return forgive(m, decision['cleared'])", 'TESTS', None,
            ['test_an_unrecordable_forgiveness_is_not_granted', 'test_an_unwritable_notice_file_forgives_nothing']),
        'corrupt-ledger-accepted': (
            'tools/parity_ratchet.py',
            "        if not (isinstance(r, dict) and type(r.get('t')) in (int, float) and math.isfinite(r['t'])",
            "        if False and (isinstance(r, dict) and type(r.get('t')) in (int, float) and math.isfinite(r['t'])",
            'TESTS', None, ['test_a_corrupt_ledger_forgives_nothing',
                            'test_a_ledger_row_with_a_non_finite_or_missing_field_forgives_nothing']),
        'ledger-accepts-a-nan-time': (
            'tools/parity_ratchet.py', " and math.isfinite(r['t'])\n", "\n", 'TESTS', None,
            ['test_a_ledger_row_with_a_non_finite_or_missing_field_forgives_nothing']),
        'confirmation-ignores-the-binary': (
            'tools/parity_ratchet.py',
            "    if not m.get('binary_sha256') or again.get('binary_sha256') != m['binary_sha256']:", "    if False:",
            'TESTS', None, ['test_a_confirmation_with_another_binary_clears_nothing']),
        'confirmation-ignores-the-denominator': (
            'tools/parity_ratchet.py', "    problems = denominators(data, again)", "    problems = []", 'TESTS', None,
            ['test_a_confirmation_with_a_moved_denominator_clears_nothing',
             'test_a_partial_census_confirmation_clears_nothing']),
        'confirmation-ignores-the-inputs': (
            'tools/parity_ratchet.py', "    if input_fingerprint(root) != fingerprint:", "    if False:", 'TESTS', None,
            ['test_inputs_that_changed_between_runs_clear_nothing']),
        'fingerprint-reads-only-names': (
            'tools/parity_ratchet.py', "+ hashlib.sha256(q.read_bytes()).digest())", ")", 'TESTS', None,
            ['test_the_fingerprint_follows_content_not_mtimes']),
        'refused-confirmation-forgives': (
            'tools/parity_ratchet.py',
            "        conf['invalid'] = f'the confirming run did not measure: {str(e)[:400]}'\n        return m",
            "        conf['invalid'] = f'the confirming run did not measure: {str(e)[:400]}'\n        return forgive(m, lost)",
            'TESTS', None, ['test_a_confirmation_that_did_not_measure_clears_nothing']),
        'check-does-not-freeze-binaries': (
            'tools/parity_ratchet.py', "            gnu_ert = freeze(gnu_ert, pathlib.Path(frozen), 'romacs-gnu-ert')",
            "            gnu_ert = gnu_ert", 'TESTS', None, ['test_every_run_uses_one_frozen_copy_of_the_binary']),
        'gnu-measurement-hides-its-binary': (
            'tools/parity_ratchet.py', "'binary_sha256': digest,\n            'seconds'",
            "'binary_sha256': None,\n            'seconds'", 'TESTS', None, ['test_the_real_adapter_is_measured']),
        # A PASS must leave its verdict in the gate's run directory.
        'pass-leaves-no-evidence': (
            'tools/parity_ratchet.py', "    if target:\n        with open(target, 'a') as f:\n            fcntl.flock(f, fcntl.LOCK_EX)\n            f.write(''.join(line + '\\n' for line in lines))\n\n\ndef confirm_losses(",
            "    pass\n\n\ndef confirm_losses(", 'TESTS', None,
            ['test_a_pass_leaves_its_verdict_as_gate_evidence', 'test_a_fail_leaves_its_verdict_as_gate_evidence']),
        'unkept-evidence-still-passes': (
            'tools/parity_ratchet.py', "                status = EXIT_NOT_MEASURED\n            worst = max(",
            "                pass\n            worst = max(", 'TESTS', None,
            ['test_a_verdict_that_cannot_be_kept_is_not_a_pass']),
        'gate-does-not-name-the-evidence-file': (
            'scripts/gate', "                test_environment['ROMACS_GATE_EVIDENCE'] = str(run_dir / EVIDENCE)\n", "",
            'GATE_TESTS', None, ['test_tests_are_told_where_to_leave_notices_and_evidence_and_stale_ones_are_dropped']),
        'gate-keeps-stale-evidence': (
            'scripts/gate', "    (run_dir / EVIDENCE).unlink(missing_ok=True)\n", "", 'GATE_TESTS', None,
            ['test_tests_are_told_where_to_leave_notices_and_evidence_and_stale_ones_are_dropped']),
        # scripts/gate must show a passing test's notices to a human.
        'gate-does-not-print-notices': (
            'scripts/gate', "    print_notices(run_dir)\n    for name in failed or ():",
            "    for name in failed or ():", 'GATE_TESTS', None,
            ['test_a_notice_from_a_passing_test_is_printed_on_green_and_red']),
        'gate-does-not-name-the-notice-file': (
            'scripts/gate', "                test_environment['ROMACS_GATE_NOTICES'] = str(run_dir / NOTICES)\n", "",
            'GATE_TESTS', None, ['test_tests_are_told_where_to_leave_notices_and_evidence_and_stale_ones_are_dropped']),
        'gate-keeps-stale-notices': (
            'scripts/gate', "    (run_dir / NOTICES).unlink(missing_ok=True)\n", "", 'GATE_TESTS', None,
            ['test_tests_are_told_where_to_leave_notices_and_evidence_and_stale_ones_are_dropped']),
        'consistency-accepts-zero-floor': (
            'tools/parity_ratchet.py', "    if not total:\n", "    if False:\n", 'TESTS', None,
            ['test_zero_floor_is_refused']),
    }


def scratch_copy(dest: pathlib.Path) -> None:
    for rel in COPY:
        src, dst = ROOT / rel, dest / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        if src.is_dir():
            shutil.copytree(src, dst, symlinks=True)
        else:
            shutil.copy2(src, dst)


def run(root, argv, args):
    cmd = [sys.executable, str(root / 'tools' / 'parity_ratchet.py'), *argv, '--root', str(root)]
    if '--gnu-ert' not in argv:
        cmd += ['--gnu-ert', args.gnu_ert]
    if '--census-bin' not in argv:
        cmd += ['--census-bin', args.census_bin]
    # A forgiveness inside a mutant run is counted in the copy, never in the
    # real ledger, and never reaches an enclosing gate's notices.
    env = {k: v for k, v in __import__('os').environ.items() if k not in ('ROMACS_GATE_NOTICES', 'ROMACS_GATE_EVIDENCE')}
    env['PARITY_RATCHET_LEDGER'] = str(root / 'parity-ratchet-unconfirmed.jsonl')
    p = subprocess.run(cmd, capture_output=True, text=True, cwd=root, env=env)
    return p.returncode, p.stdout + p.stderr


def run_tests(root, args):
    p = subprocess.run([sys.executable, str(root / 'tools' / 'parity_ratchet_test.py'), '-v'], capture_output=True,
                       text=True, cwd=root, env={**__import__('os').environ, 'ROMACS_GNU_ERT': args.gnu_ert})
    return p.returncode, p.stdout + p.stderr


def run_gate_tests(root):
    p = subprocess.run([sys.executable, str(root / 'scripts' / 'gate_test.py'), '-v'], capture_output=True,
                       text=True, cwd=root)
    return p.returncode, p.stdout + p.stderr


def gate_mutants(args) -> int:
    """Mutate THIS worktree's ratchet file / checker (never lisp/), run the gate's
    own Rust test, require it to fail at its assertion with the expected text."""
    ratchet_file, checker = ROOT / 'parity-ratchet.json', ROOT / 'tools' / 'parity_ratchet.py'
    data = json.loads(ratchet_file.read_text())
    files = sorted(data['gnu_suite']['files'])
    shard1 = [f for f in files[0::4] if len(data['gnu_suite']['passing'].get(f, [])) >= 2][0]

    def edit_floor(fn):
        d = json.loads(ratchet_file.read_text())
        fn(d)
        g = d['gnu_suite']
        g['passed'] = sum(len(v) for v in g['passing'].values())
        for mode in ('warm', 'cold'):
            d['census'][mode]['gnu_parity_loaded'] = len(d['census'][mode]['loaded'])
        return json.dumps(d, indent=1, sort_keys=True) + '\n'

    def phantom(d):
        d['gnu_suite']['passing'][shard1] = sorted(d['gnu_suite']['passing'][shard1] + ['parity-mutant-phantom'])

    def drop_one(d):
        d['gnu_suite']['passing'][shard1] = d['gnu_suite']['passing'][shard1][1:]

    def drop_cold(d):
        d['census']['cold']['loaded'] = d['census']['cold']['loaded'][1:]
    table = [
        ('gate-gnu-shard-loses-a-pass', ratchet_file, edit_floor(phantom), 'romacs-gnu-tests',
         'parity_ratchet_gnu_suite_shard_1_of_4', ['is not PASS (exit Some(1))', 'parity-mutant-phantom']),
        ('gate-gnu-shard-gains-a-pass', ratchet_file, edit_floor(drop_one), 'romacs-gnu-tests',
         'parity_ratchet_gnu_suite_shard_1_of_4', ['is not PASS (exit Some(3))', 'FAIL-WITH-INSTRUCTION']),
        ('gate-census-cold-gains-a-file', ratchet_file, edit_floor(drop_cold), 'romacs-vm',
         'parity_ratchet_census_cold', ['is not PASS (exit Some(3))', 'HIGHER census-cold']),
        ('gate-checker-test-skipped', ROOT / 'tools' / 'parity_ratchet_test.py',
         (ROOT / 'tools' / 'parity_ratchet_test.py').read_text().replace(
             "    def test_equal_is_pass(self):\n", "    def test_equal_is_pass(self):\n        self.skipTest('mutant')\n"),
         'romacs-gnu-tests', 'parity_ratchet_checker_contract', ['a checker test was skipped']),
        ('gate-pass-that-measured-nothing', checker,
         checker.read_text().replace('reported {reported} seconds', 'reported 0 seconds'), 'romacs-gnu-tests',
         'parity_ratchet_gnu_suite_shard_1_of_4', ['printed PASS but measured nothing']),
    ]
    results = []
    for name, path, mutated, pkg, test, want in table:
        before = path.read_bytes()
        try:
            if mutated.encode() == before:
                results.append((name, 'NOT-APPLIED', 'bytes unchanged'))
                continue
            path.write_bytes(mutated.encode())
            p = subprocess.run(['cargo', 'test', '--offline', '--profile', 'gate', '-p', pkg, '--test',
                                'parity_ratchet', '--', '--exact', test], cwd=ROOT, capture_output=True, text=True)
            out = p.stdout + p.stderr
            panic = next((l for l in out.splitlines() if 'panicked at' in l), '')
            missing = [w for w in want if w not in out]
            ok = p.returncode != 0 and 'parity_ratchet.rs' in panic and not missing and 'test result: FAILED. 0 passed; 1 failed' in out
            results.append((name, 'KILLED' if ok else 'SURVIVED', f'{panic.strip()}; missing {missing}'))
            if not ok:
                print(out[-3000:])
        finally:
            path.write_bytes(before)
            assert path.read_bytes() == before
    width = max(len(r[0]) for r in results)
    for name, verdict, detail in results:
        print(f'{name:<{width}}  {verdict:<11}  {detail}')
    bad = [r for r in results if r[1] != 'KILLED']
    print(f'{len(results) - len(bad)}/{len(results)} killed')
    return 1 if bad else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--gnu-ert', required=True)
    ap.add_argument('--census-bin', required=True)
    ap.add_argument('--gate', action='store_true', help='mutate this worktree and run the gate\'s Rust tests')
    ap.add_argument('--checker-only', action='store_true',
                    help='only the mutants judged by the unit tests (no product measurement, no compiled lisp/)')
    ap.add_argument('names', nargs='*')
    args = ap.parse_args()
    if args.gate:
        return gate_mutants(args)
    results = []
    with tempfile.TemporaryDirectory(prefix='parity-mutants-') as tmp:
        root = pathlib.Path(tmp) / 'root'
        scratch_copy(root)
        table = mutants(root)
        names = args.names or [n for n in table if not args.checker_only or table[n][3] in ('TESTS', 'GATE_TESTS')]
        if args.checker_only and any(table[n][3] not in ('TESTS', 'GATE_TESTS') for n in names):
            print('--checker-only names a product mutant')
            return 2
        status, out = run_tests(root, args)
        if status != 0:
            print(f'UNMUTATED copy fails its own unit tests; no checker mutant result is meaningful:\n{out[-3000:]}')
            return 1
        status, out = run_gate_tests(root)
        if status != 0:
            print(f'UNMUTATED copy fails scripts/gate_test.py; no gate mutant result is meaningful:\n{out[-3000:]}')
            return 1
        for unit in () if args.checker_only else (SUBR_FILES, 'census-cold'):
            status, out = run(root, ['check', '--unit', unit], args)
            if status != 0:
                print(f'UNMUTATED copy is not PASS on {unit} (exit {status}); no mutant result is meaningful:\n{out}')
                return 1
        for name in names:
            path, old, new, how, want_status, want_text = table[name]
            target = root / path if path else None
            before = target.read_bytes() if target else None
            try:
                if target:
                    text = before.decode()
                    if old == 'WHOLE':
                        mutated = new
                    elif old is None:
                        mutated = new + text
                    else:
                        if text.count(old) != 1:
                            results.append((name, 'NOT-APPLIED', f'old text occurs {text.count(old)} times'))
                            continue
                        mutated = text.replace(old, new)
                    target.write_bytes(mutated.encode())
                    if target.read_bytes() == before:
                        results.append((name, 'NOT-APPLIED', 'bytes unchanged'))
                        continue
                if how in ('TESTS', 'GATE_TESTS'):
                    status, out = run_tests(root, args) if how == 'TESTS' else run_gate_tests(root)
                    lines = out.splitlines()
                    # Only a failed ASSERTION is a kill: an ERROR (an exception
                    # escaping the test) proves nothing about what it protects.
                    failed = [t for t in want_text if any(line.startswith(f'FAIL: {t} ') for line in lines)]
                    ok = status != 0 and len(failed) == len(want_text)
                    detail = f'exit {status}; targeted tests failed: {failed}'
                else:
                    status, out = run(root, how, args)
                    missing = [t for t in want_text if t not in out]
                    ok = status == want_status and not missing and 'PASS ' not in out.replace('FAIL-WITH', '')
                    detail = f'exit {status} (want {want_status}); missing text {missing}'
                results.append((name, 'KILLED' if ok else 'SURVIVED', detail))
                if how != 'TESTS':
                    print(f'--- {name}:')
                    for line in out.splitlines():
                        if line.startswith(('parity-ratchet:', '    - ')):
                            print('    ' + line[:220])
                if not ok:
                    print(f'--- {name} output:\n{out[-3000:]}')
            finally:
                if target:
                    target.write_bytes(before)
                    assert target.read_bytes() == before
        for unit in () if args.checker_only else (SUBR_FILES, 'census-cold'):
            status, out = run(root, ['check', '--unit', unit], args)
            if status != 0:
                results.append(('restored-copy-passes', 'FAILED', f'{unit} exit {status}'))
    width = max(len(r[0]) for r in results)
    for name, verdict, detail in results:
        print(f'{name:<{width}}  {verdict:<11}  {detail}')
    bad = [r for r in results if r[1] != 'KILLED']
    print(f'{len(results) - len(bad)}/{len(results)} killed')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
