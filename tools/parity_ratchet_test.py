#!/usr/bin/env python3
"""The parity ratchet's own tests (run in the gate by
gnu-tests/runner/tests/parity_ratchet.rs).

    ROMACS_GNU_ERT=<adapter> python3 tools/parity_ratchet_test.py -v
"""
import contextlib
import copy
import io
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import parity_ratchet as pr  # noqa: E402
# The honest ratchet's tests run wherever these do (the gate's checker contract).
from honest_ratchet_test import HonestRatchet  # noqa: E402,F401

ROOT = pr.DEFAULT_ROOT
# Never count a test's forgiveness in the real ledger, nor print it as a gate
# notice; tests that exercise the ledger name their own file.
_LEDGER_DIR = tempfile.TemporaryDirectory(prefix='prt-ledger-')
os.environ['PARITY_RATCHET_LEDGER'] = str(pathlib.Path(_LEDGER_DIR.name) / 'suite-ledger.jsonl')
os.environ.pop('ROMACS_GATE_NOTICES', None)
os.environ.pop('ROMACS_GATE_EVIDENCE', None)


def ratchet(passing=None, warm=('lisp/a.el', 'lisp/b.el'), cold=('lisp/a.el',)):
    passing = passing if passing is not None else {
        'src/fns-tests.el': ['f1', 'f2'], 'src/data-tests.el': ['d1'], 'lisp/subr-tests.el': ['s1']}
    files = sorted(set(passing) | set(pr.CHARACTERISED) | {'lisp/other-tests.el'})
    data = {
        'schema': 1,
        'gnu_suite': {'files': files, 'file_count': len(files),
                      'tests_per_file': {f: 10 for f in files}, 'gnu_tests': 10 * len(files),
                      'passed': sum(len(v) for v in passing.values()),
                      'characterised': {f: len(passing.get(f, [])) for f in pr.CHARACTERISED},
                      'passing': {f: sorted(v) for f, v in passing.items() if v}},
        'census': {m: {'gnu_parity_loaded': len(v), 'gnu_target_total': 1596, 'shared_total': 1590,
                       'files_measured': 1811, 'chain': None, 'loaded': sorted(v)}
                   for m, v in (('warm', warm), ('cold', cold))},
        'lowerings': [],
    }
    assert pr.consistency(data) == [], pr.consistency(data)
    return data


def gnu_measurement(members, tests=10):
    files = sorted(members)
    return {'kind': 'gnu', 'files': files, 'gnu_tests': tests * len(files), 'romacs_reported': 5,
            'tests_per_file': {f: tests for f in files}, 'members': members,
            'verdicts': {f'{f}::{t}': 'errored (void-function x)' for f in files for t in ('f1', 'f2', 'd1', 's1')},
            'seconds': 0.1}


def census_measurement(mode, loaded, files_measured=1811):
    return {'kind': 'census', 'mode': mode, 'files_measured': files_measured, 'files_reported': files_measured,
            'unreported': [], 'gnu_target_total': 1596, 'shared_total': 1590,
            'members': {f'census-{mode}': loaded}, 'verdicts': {f: 'missing_variable: x' for f in loaded},
            'chain': None, 'seconds': 0.1}


def run_report(data, unit, m):
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        status = pr.report(unit, data, m)
    return status, buf.getvalue()


class Compare(unittest.TestCase):
    def test_equal_is_pass(self):
        status, out = run_report(ratchet(), 'gnu-files:x', gnu_measurement({'src/fns-tests.el': ['f1', 'f2']}))
        self.assertEqual(status, pr.EXIT_PASS, out)
        self.assertIn('parity-ratchet: PASS gnu-files:x: measured 2 floor 2', out)

    def test_a_lost_pass_is_fail_and_names_it(self):
        status, out = run_report(ratchet(), 'gnu-files:x', gnu_measurement({'src/fns-tests.el': ['f1']}))
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn('FAIL gnu:src/fns-tests.el: floor 2, measured 1 (delta -1)', out)
        self.assertIn('- f2: now errored (void-function x)', out)

    def test_a_gain_does_not_pay_for_a_loss(self):
        status, out = run_report(ratchet(), 'gnu-files:x', gnu_measurement({'src/fns-tests.el': ['f1', 'f9']}))
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn('LOST 1', out)

    def test_a_gain_is_fail_with_instruction(self):
        status, out = run_report(ratchet(), 'gnu-files:x',
                                 gnu_measurement({'src/fns-tests.el': ['f1', 'f2', 'f3']}))
        self.assertEqual(status, pr.EXIT_HIGHER, out)
        self.assertIn('FAIL-WITH-INSTRUCTION', out)
        self.assertIn('python3 tools/parity_ratchet.py record', out)

    def test_a_file_with_no_floor_that_gains_is_higher(self):
        status, out = run_report(ratchet(), 'gnu-files:x', gnu_measurement({'lisp/other-tests.el': ['o1']}))
        self.assertEqual(status, pr.EXIT_HIGHER, out)

    def test_a_moved_denominator_is_not_measured(self):
        status, out = run_report(ratchet(), 'gnu-files:x', gnu_measurement({'src/fns-tests.el': ['f1', 'f2']}, tests=9))
        self.assertEqual(status, pr.EXIT_NOT_MEASURED, out)

    def test_an_unknown_file_is_not_measured(self):
        status, out = run_report(ratchet(), 'gnu-files:x', gnu_measurement({'lisp/new-tests.el': []}))
        self.assertEqual(status, pr.EXIT_NOT_MEASURED, out)

    def test_census_lost_file_fails(self):
        status, out = run_report(ratchet(), 'census-warm', census_measurement('warm', ['lisp/a.el']))
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn('- lisp/b.el', out)

    def test_census_equal_passes_and_gain_instructs(self):
        self.assertEqual(run_report(ratchet(), 'census-cold', census_measurement('cold', ['lisp/a.el']))[0],
                         pr.EXIT_PASS)
        self.assertEqual(run_report(ratchet(), 'census-cold',
                                    census_measurement('cold', ['lisp/a.el', 'lisp/c.el']))[0], pr.EXIT_HIGHER)

    def test_nothing_reported_is_never_pass(self):
        m = gnu_measurement({'src/fns-tests.el': ['f1', 'f2']})
        m['romacs_reported'] = 0
        status, out = run_report(ratchet(), 'gnu-files:x', m)
        self.assertEqual(status, pr.EXIT_NOT_MEASURED, out)
        self.assertNotIn('PASS', out)

    def test_census_denominator_change_is_not_measured(self):
        status, out = run_report(ratchet(), 'census-cold', census_measurement('cold', ['lisp/a.el'], 1700))
        self.assertEqual(status, pr.EXIT_NOT_MEASURED, out)


class Record(unittest.TestCase):
    """record's lowering decision (apply_lowerings) and its argument guards."""

    def test_losing_a_whole_file_floor_needs_lower(self):
        old = ratchet()
        new = ratchet(passing={'src/fns-tests.el': ['f1', 'f2'], 'lisp/subr-tests.el': ['s1']})
        with self.assertRaises(pr.Refusal) as e:
            pr.apply_lowerings(old, new, {}, {})
        self.assertEqual(e.exception.status, pr.EXIT_LOWER)
        self.assertIn('gnu:src/data-tests.el: would drop 1 (d1)', str(e.exception))

    def test_a_named_lowering_is_recorded_with_its_reason(self):
        old = ratchet()
        new = ratchet(warm=('lisp/a.el',))
        pr.apply_lowerings(old, new, {'census-warm': 'lisp/ was replaced by the pristine GNU pin'}, {})
        self.assertEqual(new['lowerings'][0]['unit'], 'census-warm')
        self.assertEqual(new['lowerings'][0]['members'], ['lisp/b.el'])

    def test_lower_naming_an_unchanged_unit_is_refused(self):
        with self.assertRaises(pr.Refusal):
            pr.apply_lowerings(ratchet(), ratchet(), {'census-cold': 'nothing was lost here at all'}, {})

    def test_gains_are_reported_not_refused(self):
        raised = pr.apply_lowerings(ratchet(), ratchet(cold=('lisp/a.el', 'lisp/z.el')), {}, {})
        self.assertEqual([e['unit'] for e in raised], ['census-cold'])

    def test_one_run_is_refused(self):
        # A refused --repeat must never reach recording (which would build and
        # measure): stand cmd_record in, so the mutant fails HERE, at the assertion.
        saved, pr.cmd_record = pr.cmd_record, lambda args: 'recording was reached'
        try:
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(pr.main(['record', '--repeat', '1']), pr.EXIT_INVALID)
        finally:
            pr.cmd_record = saved

    def test_only_python_bytecode_is_harmless(self):
        self.assertTrue(pr.python_cache('scripts/__pycache__/'))
        self.assertTrue(pr.python_cache('scripts/__pycache__/x.cpython-313.pyc'))
        self.assertFalse(pr.python_cache('lisp/__pycache__/'))
        self.assertFalse(pr.python_cache('lisp/international/charscript.el'))
        self.assertFalse(pr.python_cache('scripts/__pycache__/gen.el'))


class Confirmation(unittest.TestCase):
    """merge_confirmation (pure): only a confirming run that meets the unit's
    floor by itself clears a loss, and it restores only what was lost."""

    def test_a_reproduced_loss_stays_lost(self):
        first = gnu_measurement({'src/fns-tests.el': ['f1']})
        decision = pr.merge_confirmation(ratchet(), first, gnu_measurement({'src/fns-tests.el': ['f1']}))
        self.assertEqual(decision, {'cleared': {}, 'reproduced': {'gnu:src/fns-tests.el': ['f2']}})

    def test_a_loss_that_does_not_reproduce_is_cleared(self):
        first = gnu_measurement({'src/fns-tests.el': ['f1']})
        decision = pr.merge_confirmation(ratchet(), first, gnu_measurement({'src/fns-tests.el': ['f1', 'f2']}))
        self.assertEqual(decision['cleared'], {'gnu:src/fns-tests.el': ['f2']})
        self.assertEqual(run_report(ratchet(), 'gnu-files:x', pr.forgive(first, decision['cleared']))[0], pr.EXIT_PASS)

    def test_confirmation_never_adds_gains(self):
        first = gnu_measurement({'src/fns-tests.el': ['f1']})
        decision = pr.merge_confirmation(ratchet(), first, gnu_measurement({'src/fns-tests.el': ['f1', 'f2', 'f9']}))
        self.assertEqual(pr.forgive(first, decision['cleared'])['members']['src/fns-tests.el'], ['f1', 'f2'])

    def test_a_different_member_lost_in_the_confirmation_clears_nothing(self):
        first = gnu_measurement({'src/fns-tests.el': ['f1']})
        decision = pr.merge_confirmation(ratchet(), first, gnu_measurement({'src/fns-tests.el': ['f2']}))
        self.assertEqual(decision, {'cleared': {}, 'reproduced': {'gnu:src/fns-tests.el': ['f1']}})

    def test_census_losses_are_confirmed_against_the_whole_floor(self):
        first = census_measurement('warm', ['lisp/a.el'])
        self.assertEqual(pr.lost_members(ratchet(), first), {'census-warm': ['lisp/b.el']})
        decision = pr.merge_confirmation(ratchet(), first, census_measurement('warm', ['lisp/b.el']))
        self.assertEqual(decision['reproduced'], {'census-warm': ['lisp/a.el']})
        decision = pr.merge_confirmation(ratchet(), first, census_measurement('warm', ['lisp/a.el', 'lisp/b.el']))
        self.assertEqual(pr.forgive(first, decision['cleared'])['members']['census-warm'], ['lisp/a.el', 'lisp/b.el'])


FNS = 'src/fns-tests.el'


def fns_members():
    """Two members src/fns-tests.el records RIGHT NOW. Hardcoding a member NAME
    has exactly the defect fns_floor() documents for the floor SIZE: when the
    recorded set changes, a correct ratchet turns into a red gate. Measured: the
    owner's 2026-10-22 floor restatement dropped fns-tests-nreverse (16 members
    -> 13) and every test naming it failed in its own fixture, not in the
    checker. Derive the names; never spell them."""
    floor = sorted(pr.load_ratchet(ROOT, evidence=False)['gnu_suite']['passing'][FNS])
    assert len(floor) >= 2, floor
    return floor[0], floor[1]


LOST, LOST2 = fns_members()


def check_with_runs(runs, unit=f'gnu-files:{FNS}'):
    """cmd_check on the REAL ratchet file, with each measurement replaced, in
    order, by RUNS (the first run, then every confirmation)."""
    queue = list(runs)
    calls = []

    def fake(*a, **k):
        calls.append(a)
        if not queue:
            raise AssertionError('more measurements were taken than supplied')
        return copy.deepcopy(queue.pop(0))
    tmp = tempfile.TemporaryDirectory(prefix='prt-bin-')
    binary = pathlib.Path(tmp.name) / 'romacs-gnu-ert'
    binary.write_bytes(b'not executed: every measurement is supplied')
    args = type('A', (), {'root': str(ROOT), 'tier': None, 'unit': unit, 'gnu_ert': str(binary),
                          'census_bin': str(binary), 'jobs': 1})()
    saved = pr.measure_unit, pr.measure_gnu, pr.measure_census
    pr.measure_unit = pr.measure_gnu = pr.measure_census = fake
    out = io.StringIO()
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            try:
                status = pr.cmd_check(args)
            except pr.Refusal as e:
                status = e.status
                print(f'REFUSED {e}')
    finally:
        pr.measure_unit, pr.measure_gnu, pr.measure_census = saved
        tmp.cleanup()
    return status, out.getvalue(), len(calls) - len(queue)


def fns_floor():
    """How many members src/fns-tests.el records RIGHT NOW. A branch that wins
    more of this file raises it, and a test that hardcoded the old size turned
    a legitimate gain into a red gate (measured: floor 14 -> 16)."""
    return len(pr.load_ratchet(ROOT)['gnu_suite']['passing'][FNS])


def real_fns_run(missing):
    """A GNU measurement of src/fns-tests.el: the recorded floor minus MISSING."""
    data = pr.load_ratchet(ROOT)
    floor = data['gnu_suite']['passing'][FNS]
    # The floor RISES as parity improves, so no expectation may hardcode its
    # size: fns_floor() is the only source for the numbers the verdicts print.
    assert set(missing) <= set(floor), missing
    m = gnu_measurement({FNS: sorted(set(floor) - set(missing))}, tests=data['gnu_suite']['tests_per_file'][FNS])
    m['verdicts'] = {f'{FNS}::{t}': 'failed' for t in missing}
    m['binary_sha256'] = '0' * 64
    return m


class AlternatingLoss(unittest.TestCase):
    """The reviewer's counterexample (REJECT of 13b4d4aa): two runs that each
    FAIL the real src/fns-tests.el floor, each losing a DIFFERENT member, were
    merged into a PASS that exited 0."""

    def test_two_runs_that_each_lose_a_different_member_are_not_a_pass(self):
        first = real_fns_run([LOST])
        second = real_fns_run([LOST2])
        status, out, _ = check_with_runs([first, second])
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertNotIn(f'PASS gnu-files:{FNS}', out)
        self.assertIn(LOST, out)
        self.assertIn(LOST2, out)


def real_census_run(mode, missing=(), files_measured=None):
    data = pr.load_ratchet(ROOT)
    c = data['census'][mode]
    m = census_measurement(mode, sorted(set(c['loaded']) - set(missing)), files_measured or c['files_measured'])
    m.update(gnu_target_total=c['gnu_target_total'], shared_total=c['shared_total'], binary_sha256='0' * 64)
    return m


class CheckConfirmation(unittest.TestCase):
    """cmd_check end to end on the real floors: what a confirmation may clear,
    and that every clearance is visible, counted and escalated."""

    def setUp(self):
        tmp = tempfile.TemporaryDirectory(prefix='prt-ledger-')
        self.addCleanup(tmp.cleanup)
        self.dir = pathlib.Path(tmp.name)
        self.ledger = self.dir / 'ledger.jsonl'
        self.notices = self.dir / 'notices.txt'
        env = {'PARITY_RATCHET_LEDGER': str(self.ledger), 'ROMACS_GATE_NOTICES': str(self.notices)}
        saved = {k: os.environ.get(k) for k in env}
        os.environ.update(env)
        self.addCleanup(lambda: [os.environ.pop(k, None) if v is None else os.environ.__setitem__(k, v)
                                 for k, v in saved.items()])

    def noticed(self):
        return self.notices.read_text() if self.notices.is_file() else ''

    def rows(self):
        return [json.loads(line) for line in self.ledger.read_text().splitlines()] if self.ledger.exists() else []

    def test_a_member_absent_in_both_runs_fails(self):
        status, out, _ = check_with_runs([real_fns_run([LOST]), real_fns_run([LOST])])
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn(f'confirmation run also missed the floor: lost {LOST}', out)
        self.assertEqual(self.rows(), [])

    def test_a_cleared_loss_passes_visibly_counted_and_noticed(self):
        status, out, measured = check_with_runs([real_fns_run([LOST]), real_fns_run([])])
        self.assertEqual((status, measured), (pr.EXIT_PASS, 2), out)
        n = fns_floor()
        self.assertIn(f'PASS gnu-files:{FNS}: measured {n} floor {n}', out)
        self.assertIn('unconfirmed 1', out)
        self.assertIn(f'UNCONFIRMED gnu:{FNS}: 1 floor member(s) lost in the first run', out)
        self.assertEqual([(r['unit'], r['member'], r['outcome']) for r in self.rows()],
                         [(f'gnu:{FNS}', LOST, 'forgiven')])
        self.assertIn(f'UNCONFIRMED gnu:{FNS}', self.noticed())

    def evidence(self, runs):
        path = self.dir / 'evidence.txt'
        os.environ['ROMACS_GATE_EVIDENCE'] = str(path)
        try:
            status, out, _ = check_with_runs(runs)
        finally:
            os.environ.pop('ROMACS_GATE_EVIDENCE')
        return status, out, path.read_text() if path.is_file() else ''

    def test_a_pass_leaves_its_verdict_as_gate_evidence(self):
        status, out, kept = self.evidence([real_fns_run([LOST]), real_fns_run([])])
        self.assertEqual(status, pr.EXIT_PASS, out)
        verdict = next(l for l in out.splitlines() if l.startswith(f'parity-ratchet: PASS gnu-files:{FNS}:'))
        n = fns_floor()
        self.assertIn(f'measured {n} floor {n} units 1 denominator 95 reported 5', verdict)
        self.assertIn(verdict, kept.splitlines())
        self.assertIn(f'parity-ratchet: UNCONFIRMED gnu:{FNS}', kept)

    def test_a_fail_leaves_its_verdict_as_gate_evidence(self):
        status, out, kept = self.evidence([real_fns_run([LOST]), real_fns_run([LOST])])
        self.assertEqual(status, pr.EXIT_LOWER, out)
        n = fns_floor()
        self.assertIn(f'parity-ratchet: FAIL gnu-files:{FNS}: measured {n - 1} floor {n}', kept)

    def test_a_verdict_that_cannot_be_kept_is_not_a_pass(self):
        (self.dir / 'evidence.txt').mkdir()
        status, out, _ = self.evidence([real_fns_run([])])
        self.assertEqual(status, pr.EXIT_NOT_MEASURED, out)
        self.assertIn('could not be kept as gate evidence', out)

    def test_the_ledger_can_be_read_by_a_human(self):
        check_with_runs([real_fns_run([LOST]), real_fns_run([])])
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(pr.main(['unconfirmed']), 0)
        self.assertIn(f'FORGIVEN  gnu:{FNS} {LOST} first run: failed', out.getvalue())
        self.assertIn(f'gnu:{FNS} {LOST} x1', out.getvalue())

    def test_the_unconfirmed_line_claims_no_cause(self):
        _, out, _ = check_with_runs([real_fns_run([LOST]), real_fns_run([])])
        self.assertNotIn('not a regression', out)
        self.assertNotIn('was disturbed', out)
        self.assertIn('Cause NOT established', out)

    def test_the_same_member_forgiven_again_escalates_to_fail(self):
        runs = [real_fns_run([LOST]), real_fns_run([])]
        self.assertEqual(check_with_runs(runs)[0], pr.EXIT_PASS)
        status, out, _ = check_with_runs(runs)
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn(f'ESCALATED: {LOST} was already forgiven 1 time(s)', out)
        self.assertEqual([r['outcome'] for r in self.rows()], ['forgiven', 'escalated'])
        self.assertIn(f'ESCALATED gnu:{FNS}', self.noticed())

    def test_a_forgiveness_outside_the_window_does_not_count(self):
        old = time.time() - (pr.UNCONFIRMED_WINDOW_DAYS + 1) * 86400
        self.ledger.write_text(json.dumps({'t': old, 'unit': f'gnu:{FNS}', 'member': LOST}) + '\n')
        status, out, _ = check_with_runs([real_fns_run([LOST]), real_fns_run([])])
        self.assertEqual(status, pr.EXIT_PASS, out)

    def test_another_members_forgiveness_does_not_count(self):
        self.ledger.write_text(json.dumps({'t': time.time(), 'unit': f'gnu:{FNS}', 'member': 'test-remhash'}) + '\n')
        status, out, _ = check_with_runs([real_fns_run([LOST]), real_fns_run([])])
        self.assertEqual(status, pr.EXIT_PASS, out)

    def test_an_unrecordable_forgiveness_is_not_granted(self):
        self.ledger.mkdir()
        status, out, _ = check_with_runs([real_fns_run([LOST]), real_fns_run([])])
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn('could not be recorded', out)

    def test_a_corrupt_ledger_forgives_nothing(self):
        self.ledger.write_text('{"unit": "no time"}\n')
        status, out, _ = check_with_runs([real_fns_run([LOST]), real_fns_run([])])
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn('could not be recorded', out)

    def test_a_ledger_row_with_a_non_finite_or_missing_field_forgives_nothing(self):
        prior = {'unit': f'gnu:{FNS}', 'member': LOST}
        for row in ('{"t": NaN, "unit": "gnu:%s", "member": "%s"}' % (FNS, LOST),
                    '{"t": Infinity, "unit": "gnu:%s", "member": "%s"}' % (FNS, LOST),
                    '{"t": true, "unit": "gnu:%s", "member": "%s"}' % (FNS, LOST),
                    json.dumps({'t': time.time(), 'unit': prior['unit']})):
            with self.subTest(row=row):
                self.ledger.write_text(row + '\n')
                status, out, _ = check_with_runs([real_fns_run([LOST]), real_fns_run([])])
                self.assertEqual(status, pr.EXIT_LOWER, out)
                self.assertIn('could not be recorded', out)

    def test_an_unwritable_notice_file_forgives_nothing(self):
        self.notices.mkdir()
        status, out, _ = check_with_runs([real_fns_run([LOST]), real_fns_run([])])
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn('could not be recorded', out)

    def test_a_confirmation_with_another_binary_clears_nothing(self):
        second = real_fns_run([])
        second['binary_sha256'] = '1' * 64
        status, out, _ = check_with_runs([real_fns_run([LOST]), second])
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn('the confirming run used another binary', out)

    def test_a_confirmation_with_a_moved_denominator_clears_nothing(self):
        second = real_fns_run([])
        second['tests_per_file'] = {FNS: second['tests_per_file'][FNS] - 1}
        status, out, _ = check_with_runs([real_fns_run([LOST]), second])
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn('different denominator', out)

    def test_a_partial_census_confirmation_clears_nothing(self):
        loaded = pr.load_ratchet(ROOT)['census']['warm']['loaded']
        partial = real_census_run('warm', files_measured=1)
        status, out, _ = check_with_runs([real_census_run('warm', [loaded[0]]), partial], unit='census-warm')
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn('different denominator', out)

    def test_a_whole_census_confirmation_clears(self):
        loaded = pr.load_ratchet(ROOT)['census']['warm']['loaded']
        status, out, _ = check_with_runs([real_census_run('warm', [loaded[0]]), real_census_run('warm')],
                                         unit='census-warm')
        self.assertEqual(status, pr.EXIT_PASS, out)
        self.assertIn('UNCONFIRMED census-warm', out)

    def test_inputs_that_changed_between_runs_clear_nothing(self):
        saved = pr.input_fingerprint
        values = iter(['before', 'after'])
        pr.input_fingerprint = lambda root: next(values)
        try:
            status, out, _ = check_with_runs([real_fns_run([LOST]), real_fns_run([])])
        finally:
            pr.input_fingerprint = saved
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn('the measured inputs changed', out)

    def test_a_confirmation_that_did_not_measure_clears_nothing(self):
        def refuse(*a):
            raise pr.Refusal(pr.EXIT_NOT_MEASURED, 'run.py was killed by signal 9')
        data, first, saved = pr.load_ratchet(ROOT), real_fns_run([LOST]), pr.measure_gnu
        pr.measure_gnu = refuse
        try:
            m = pr.confirm_losses(ROOT, data, first, '/frozen/adapter', None, 1, pr.input_fingerprint(ROOT))
        finally:
            pr.measure_gnu = saved
        status, out = run_report(data, f'gnu-files:{FNS}', m)
        self.assertEqual(status, pr.EXIT_LOWER, out)
        self.assertIn('the confirming run did not measure', out)

    def test_every_run_uses_one_frozen_copy_of_the_binary(self):
        queue = [real_fns_run([LOST]), real_fns_run([])]
        seen = []

        def fake(*a):
            seen.append(a[2])
            self.assertTrue(pathlib.Path(a[2]).is_file(), a[2])
            return copy.deepcopy(queue.pop(0))
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        binary = pathlib.Path(tmp.name) / 'adapter'
        binary.write_bytes(b'adapter bytes')
        saved = pr.measure_unit, pr.measure_gnu
        pr.measure_unit = pr.measure_gnu = fake
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                pr.cmd_check(type('A', (), {'root': str(ROOT), 'tier': None, 'unit': f'gnu-files:{FNS}',
                                            'gnu_ert': str(binary), 'census_bin': None, 'jobs': 1})())
        finally:
            pr.measure_unit, pr.measure_gnu = saved
        self.assertEqual(len(seen), 2)
        self.assertEqual(seen[0], seen[1])
        self.assertNotEqual(seen[0], str(binary))


class Provenance(unittest.TestCase):
    def test_the_fingerprint_follows_content_not_mtimes(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = pathlib.Path(tmp.name)
        (root / 'gnu-lisp').mkdir()
        (root / 'gnu-lisp' / 'a.el').write_text('(a)')
        before = pr.input_fingerprint(root)
        os.utime(root / 'gnu-lisp' / 'a.el', (1, 1))
        self.assertEqual(pr.input_fingerprint(root), before)
        (root / 'gnu-lisp' / 'a.el').write_text('(b)')
        self.assertNotEqual(pr.input_fingerprint(root), before)
        (root / 'gnu-lisp' / 'a.el').write_text('(a)')
        (root / 'gnu-lisp' / 'new.el').write_text('')
        self.assertNotEqual(pr.input_fingerprint(root), before)

    def test_the_ledger_ignores_a_private_home(self):
        saved = os.environ.pop('PARITY_RATCHET_LEDGER'), os.environ.get('HOME')
        os.environ['HOME'] = '/nonexistent/private-gate-home'
        try:
            path = pr.ledger_path()
        finally:
            os.environ['PARITY_RATCHET_LEDGER'], os.environ['HOME'] = saved
        self.assertFalse(str(path).startswith('/nonexistent'), path)
        self.assertEqual(path.name, pr.LEDGER_NAME)


class Consistency(unittest.TestCase):
    def test_zero_floor_is_refused(self):
        data = ratchet()
        data['gnu_suite']['passing'] = {}
        data['gnu_suite']['passed'] = 0
        data['gnu_suite']['characterised'] = {f: 0 for f in pr.CHARACTERISED}
        self.assertTrue(any('zero' in p for p in pr.consistency(data)))

    def test_count_must_match_members(self):
        data = ratchet()
        data['census']['warm']['gnu_parity_loaded'] = 3
        self.assertTrue(pr.consistency(data))
        data = ratchet()
        data['gnu_suite']['passed'] += 1
        self.assertTrue(pr.consistency(data))

    def test_lowering_needs_a_reason(self):
        data = ratchet()
        data['lowerings'] = [{'unit': 'census-warm', 'members': ['lisp/b.el'], 'because': 'oops'}]
        self.assertTrue(pr.consistency(data))


class Verify(unittest.TestCase):
    """verify compares against a base revision through git; build a tiny repo."""

    def repo(self, base, head, branch=()):
        tmp = tempfile.TemporaryDirectory(prefix='prt-verify-')
        self.addCleanup(tmp.cleanup)
        root = pathlib.Path(tmp.name)
        env = dict(os.environ, GIT_AUTHOR_NAME='t', GIT_AUTHOR_EMAIL='t@t', GIT_COMMITTER_NAME='t',
                   GIT_COMMITTER_EMAIL='t@t', GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_SYSTEM='/dev/null')
        def git(*a):
            subprocess.run(['git', '-C', str(root), *a], check=True, capture_output=True, env=env)
        git('init', '-q', '-b', 'master')
        for rel in pr.GATE_TESTS:  # the gate wiring verify also checks
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text((ROOT / rel).read_text())
        (root / pr.RATCHET_NAME).write_text(json.dumps(base))
        git('add', '.')
        git('commit', '-q', '--no-gpg-sign', '-m', 'base')
        git('checkout', '-q', '-b', 'lane')
        for i, version in enumerate(branch):  # recordings committed on the lane
            (root / pr.RATCHET_NAME).write_text(json.dumps(version))
            git('commit', '-q', '--no-gpg-sign', '-am', f'lane {i}')
        (root / pr.RATCHET_NAME).write_text(json.dumps(head))
        return root

    def verify(self, root, base=None):
        err = io.StringIO()
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
            status = pr.cmd_verify(type('A', (), {'root': str(root), 'base': base})())
        return status, err.getvalue()

    def test_a_floor_raised_on_the_lane_cannot_be_dropped_later(self):
        raised = ratchet(warm=('lisp/a.el', 'lisp/b.el', 'lisp/c.el'))
        status, err = self.verify(self.repo(ratchet(), ratchet(), branch=[raised]))
        self.assertEqual(status, pr.EXIT_LOWER, err)
        self.assertIn('census-warm: 1 dropped (lisp/c.el)', err)

    def test_an_old_lowering_record_does_not_authorize_a_new_loss(self):
        base = ratchet()  # lisp/b.el was lowered once and has since been restored
        base['lowerings'] = [{'unit': 'census-warm', 'members': ['lisp/b.el'],
                              'because': 'lisp/ was replaced by the pristine GNU pin'}]
        head = copy.deepcopy(base)
        head['census']['warm']['loaded'] = ['lisp/a.el']
        head['census']['warm']['gnu_parity_loaded'] = 1
        status, err = self.verify(self.repo(base, head))
        self.assertEqual(status, pr.EXIT_LOWER, err)
        self.assertIn('lisp/b.el', err)

    def test_a_bad_base_is_refused_not_passed_over(self):
        root = self.repo(ratchet(), ratchet())
        with self.assertRaises(pr.Refusal):
            self.verify(root, base='not-a-valid-base')

    def test_deleting_the_file_on_the_lane_is_refused(self):
        root = self.repo(ratchet(), ratchet())
        status, err = None, ''
        subprocess.run(['git', '-C', str(root), 'rm', '-q', pr.RATCHET_NAME], check=True)
        subprocess.run(['git', '-C', str(root), '-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-q',
                        '--no-gpg-sign', '-m', 'drop'], check=True, env=dict(os.environ, GIT_CONFIG_GLOBAL='/dev/null'))
        (root / pr.RATCHET_NAME).write_text(json.dumps(ratchet()))
        status, err = self.verify(root)
        self.assertEqual(status, pr.EXIT_LOWER, err)
        self.assertIn('was deleted', err)

    def test_silent_lowering_is_refused(self):
        head = ratchet(warm=('lisp/a.el',))
        status, err = self.verify(self.repo(ratchet(), head))
        self.assertEqual(status, pr.EXIT_LOWER, err)
        self.assertIn('census-warm: 1 dropped (lisp/b.el)', err)

    def test_recorded_lowering_is_accepted(self):
        head = ratchet(warm=('lisp/a.el',))
        head['lowerings'] = [{'unit': 'census-warm', 'members': ['lisp/b.el'],
                              'because': 'lisp/ was replaced by the pristine GNU pin'}]
        self.assertEqual(self.verify(self.repo(ratchet(), head))[0], 0)

    def test_deleting_a_lowering_record_is_refused(self):
        base = ratchet(warm=('lisp/a.el',))
        base['lowerings'] = [{'unit': 'census-warm', 'members': ['lisp/b.el'],
                              'because': 'lisp/ was replaced by the pristine GNU pin'}]
        head = copy.deepcopy(base)
        head['lowerings'] = []
        self.assertEqual(self.verify(self.repo(base, head))[0], pr.EXIT_LOWER)

    def test_raising_is_accepted(self):
        head = ratchet(warm=('lisp/a.el', 'lisp/b.el', 'lisp/c.el'))
        self.assertEqual(self.verify(self.repo(ratchet(), head))[0], 0)

    def test_dropping_a_whole_file_floor_is_refused(self):
        head = ratchet(passing={'src/fns-tests.el': ['f1', 'f2'], 'lisp/subr-tests.el': ['s1']})
        status, err = self.verify(self.repo(ratchet(), head))
        self.assertEqual(status, pr.EXIT_LOWER, err)
        self.assertIn('gnu:src/data-tests.el', err)


class VerifyForwardMerge(unittest.TestCase):
    """A lane that branched BEFORE the ratchet existed, then merged master
    forward: its own pre-merge commits never had the file and are not steps
    of the ratchet's history (they blocked every landing as 'deleted')."""

    def lane(self, master_floor, after_merge=None, merge_floor=None):
        tmp = tempfile.TemporaryDirectory(prefix='prt-forward-')
        self.addCleanup(tmp.cleanup)
        root = pathlib.Path(tmp.name)
        env = dict(os.environ, GIT_AUTHOR_NAME='t', GIT_AUTHOR_EMAIL='t@t', GIT_COMMITTER_NAME='t',
                   GIT_COMMITTER_EMAIL='t@t', GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_SYSTEM='/dev/null')

        def git(*a):
            subprocess.run(['git', '-C', str(root), *a], check=True, capture_output=True, env=env)
        git('init', '-q', '-b', 'master')
        (root / 'README').write_text('before the ratchet\n')
        git('add', '.')
        git('commit', '-q', '--no-gpg-sign', '-m', 'pre-ratchet')
        git('checkout', '-q', '-b', 'lane')
        for i in range(3):  # the lane's own work, from before the ratchet
            (root / f'lane-{i}').write_text(str(i))
            git('add', '.')
            git('commit', '-q', '--no-gpg-sign', '-m', f'lane {i}')
        git('checkout', '-q', 'master')
        for rel in pr.GATE_TESTS:
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text((ROOT / rel).read_text())
        (root / pr.RATCHET_NAME).write_text(json.dumps(master_floor))
        git('add', '.')
        git('commit', '-q', '--no-gpg-sign', '-m', 'the ratchet lands on master')
        git('checkout', '-q', 'lane')
        git('merge', '-q', '--no-ff', '--no-gpg-sign', '--no-commit', 'master')
        if merge_floor is not None:  # the merge itself resolves to another file
            (root / pr.RATCHET_NAME).write_text(json.dumps(merge_floor))
            git('add', pr.RATCHET_NAME)
        git('commit', '-q', '--no-gpg-sign', '-m', 'merge master forward')
        if after_merge is not None:
            (root / pr.RATCHET_NAME).write_text(json.dumps(after_merge))
            git('commit', '-q', '--no-gpg-sign', '-am', 'a lane commit after the merge')
        return root

    def verify(self, root):
        return Verify.verify(self, root)

    def raised_then_dropped_before_a_forward_merge(self):
        """Sol's counterexample: the lane exists after the ratchet, raises a
        floor, silently drops it again, then merges a moved master forward."""
        tmp = tempfile.TemporaryDirectory(prefix='prt-forward-')
        self.addCleanup(tmp.cleanup)
        root = pathlib.Path(tmp.name)
        env = dict(os.environ, GIT_AUTHOR_NAME='t', GIT_AUTHOR_EMAIL='t@t', GIT_COMMITTER_NAME='t',
                   GIT_COMMITTER_EMAIL='t@t', GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_SYSTEM='/dev/null')

        def git(*a):
            subprocess.run(['git', '-C', str(root), *a], check=True, capture_output=True, env=env)
        git('init', '-q', '-b', 'master')
        for rel in pr.GATE_TESTS:
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text((ROOT / rel).read_text())
        (root / pr.RATCHET_NAME).write_text(json.dumps(ratchet()))
        git('add', '.')
        git('commit', '-q', '--no-gpg-sign', '-m', 'ratchet on master')
        git('checkout', '-q', '-b', 'lane')
        (root / pr.RATCHET_NAME).write_text(json.dumps(ratchet(warm=('lisp/a.el', 'lisp/b.el', 'lisp/c.el'))))
        git('commit', '-q', '--no-gpg-sign', '-am', 'lane raises warm by lisp/c.el')
        (root / pr.RATCHET_NAME).write_text(json.dumps(ratchet()))
        git('commit', '-q', '--no-gpg-sign', '-am', 'lane silently drops lisp/c.el')
        git('checkout', '-q', 'master')
        (root / 'unrelated').write_text('x')
        git('add', '.')
        git('commit', '-q', '--no-gpg-sign', '-m', 'master moves')
        git('checkout', '-q', 'lane')
        git('merge', '-q', '--no-ff', '--no-gpg-sign', '-m', 'merge master forward', 'master')
        return root

    def test_a_floor_raised_and_dropped_on_a_merged_side_branch_is_refused(self):
        """Sol's second counterexample: the raise and the silent drop both
        happen on a side branch that the lane then merges."""
        tmp = tempfile.TemporaryDirectory(prefix='prt-side-')
        self.addCleanup(tmp.cleanup)
        root = pathlib.Path(tmp.name)
        env = dict(os.environ, GIT_AUTHOR_NAME='t', GIT_AUTHOR_EMAIL='t@t', GIT_COMMITTER_NAME='t',
                   GIT_COMMITTER_EMAIL='t@t', GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_SYSTEM='/dev/null')

        def git(*a):
            subprocess.run(['git', '-C', str(root), *a], check=True, capture_output=True, env=env)
        git('init', '-q', '-b', 'master')
        for rel in pr.GATE_TESTS:
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text((ROOT / rel).read_text())
        (root / pr.RATCHET_NAME).write_text(json.dumps(ratchet()))
        git('add', '.')
        git('commit', '-q', '--no-gpg-sign', '-m', 'base')
        git('checkout', '-q', '-b', 'lane')
        git('checkout', '-q', '-b', 'side')
        (root / pr.RATCHET_NAME).write_text(json.dumps(ratchet(warm=('lisp/a.el', 'lisp/b.el', 'lisp/c.el'))))
        git('commit', '-q', '--no-gpg-sign', '-am', 'record improvement')
        (root / pr.RATCHET_NAME).write_text(json.dumps(ratchet()))
        git('commit', '-q', '--no-gpg-sign', '-am', 'drop floor silently')
        git('checkout', '-q', 'lane')
        (root / 'work').write_text('x')
        git('add', '.')
        git('commit', '-q', '--no-gpg-sign', '-m', 'ordinary lane work')
        git('merge', '-q', '--no-ff', '--no-gpg-sign', '-m', 'integrate side', 'side')
        status, err = self.verify(root)
        self.assertEqual(status, pr.EXIT_LOWER, err)
        self.assertIn('census-warm: 1 dropped (lisp/c.el)', err)

    def two_recorded_lowerings(self, order, keep=('warm', 'cold')):
        """Lane records a warm lowering, a side branch a cold one; the lane
        merges the side keeping both floors as lowered and the records KEEP."""
        tmp = tempfile.TemporaryDirectory(prefix='prt-lowerings-')
        self.addCleanup(tmp.cleanup)
        root = pathlib.Path(tmp.name)
        env = dict(os.environ, GIT_AUTHOR_NAME='t', GIT_AUTHOR_EMAIL='t@t', GIT_COMMITTER_NAME='t',
                   GIT_COMMITTER_EMAIL='t@t', GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_SYSTEM='/dev/null')

        def git(*a):
            subprocess.run(['git', '-C', str(root), *a], check=True, capture_output=True, env=env)
        base = ratchet(warm=('lisp/a.el', 'lisp/b.el'), cold=('lisp/a.el', 'lisp/b.el'))
        warm_low = {'unit': 'census-warm', 'members': ['lisp/b.el'], 'because': 'lisp/ was replaced by the GNU pin'}
        cold_low = {'unit': 'census-cold', 'members': ['lisp/b.el'], 'because': 'the cold chain was replaced wholesale'}
        lane = ratchet(warm=('lisp/a.el',), cold=('lisp/a.el', 'lisp/b.el'))
        lane['lowerings'] = [warm_low]
        side = ratchet(warm=('lisp/a.el', 'lisp/b.el'), cold=('lisp/a.el',))
        side['lowerings'] = [cold_low]
        merged = ratchet(warm=('lisp/a.el',), cold=('lisp/a.el',))
        merged['lowerings'] = [{'warm': warm_low, 'cold': cold_low}[k] for k in order if k in keep]
        git('init', '-q', '-b', 'master')
        for rel in pr.GATE_TESTS:
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text((ROOT / rel).read_text())
        (root / pr.RATCHET_NAME).write_text(json.dumps(base))
        git('add', '.')
        git('commit', '-q', '--no-gpg-sign', '-m', 'base')
        git('checkout', '-q', '-b', 'side')
        (root / pr.RATCHET_NAME).write_text(json.dumps(side))
        git('commit', '-q', '--no-gpg-sign', '-am', 'side records a cold lowering')
        git('checkout', '-q', '-b', 'lane', 'master')
        (root / pr.RATCHET_NAME).write_text(json.dumps(lane))
        git('commit', '-q', '--no-gpg-sign', '-am', 'lane records a warm lowering')
        git('merge', '-q', '--no-ff', '--no-gpg-sign', '--no-commit', '-X', 'ours', 'side')
        (root / pr.RATCHET_NAME).write_text(json.dumps(merged))
        git('add', pr.RATCHET_NAME)
        git('commit', '-q', '--no-gpg-sign', '-m', 'merge side')
        return root

    def test_independent_recorded_lowerings_merge_in_either_order(self):
        for order in (('warm', 'cold'), ('cold', 'warm')):
            with self.subTest(order=order):
                status, err = self.verify(self.two_recorded_lowerings(order))
                self.assertEqual(status, 0, err)

    def test_a_merge_that_deletes_a_parents_lowering_record_is_refused(self):
        status, err = self.verify(self.two_recorded_lowerings(('warm', 'cold'), keep=('warm',)))
        self.assertEqual(status, pr.EXIT_LOWER, err)
        self.assertIn('earlier lowering records were changed or deleted', err)

    def test_a_lane_floor_dropped_before_a_forward_merge_is_still_refused(self):
        status, err = self.verify(self.raised_then_dropped_before_a_forward_merge())
        self.assertEqual(status, pr.EXIT_LOWER, err)
        self.assertIn('census-warm: 1 dropped (lisp/c.el)', err)


    def test_pre_ratchet_lane_commits_are_not_deletions(self):
        status, err = self.verify(self.lane(ratchet()))
        self.assertEqual(status, 0, err)
        self.assertNotIn('was deleted', err)

    def test_a_lowering_after_the_forward_merge_is_still_refused(self):
        status, err = self.verify(self.lane(ratchet(), after_merge=ratchet(warm=('lisp/a.el',))))
        self.assertEqual(status, pr.EXIT_LOWER, err)
        self.assertIn('census-warm: 1 dropped (lisp/b.el)', err)

    def test_a_lowering_inside_the_forward_merge_is_still_refused(self):
        status, err = self.verify(self.lane(ratchet(), merge_floor=ratchet(warm=('lisp/a.el',))))
        self.assertEqual(status, pr.EXIT_LOWER, err)
        self.assertIn('census-warm: 1 dropped (lisp/b.el)', err)


class VerifyCandidateMerge(unittest.TestCase):
    """A candidate merges a branch that forked before a lowering master
    recorded: the merge keeps master's file, and the branch's step to it is
    master's own history, unless the branch changed the file itself."""

    def repo(self, side_raises):
        tmp = tempfile.TemporaryDirectory(prefix='prt-candidate-')
        self.addCleanup(tmp.cleanup)
        root = pathlib.Path(tmp.name)
        env = dict(os.environ, GIT_AUTHOR_NAME='t', GIT_AUTHOR_EMAIL='t@t', GIT_COMMITTER_NAME='t',
                   GIT_COMMITTER_EMAIL='t@t', GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_SYSTEM='/dev/null')

        def git(*a):
            return subprocess.run(['git', '-C', str(root), *a], check=True, capture_output=True, env=env,
                                  text=True).stdout.strip()
        git('init', '-q', '-b', 'master')
        for rel in pr.GATE_TESTS:
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text((ROOT / rel).read_text())
        (root / pr.RATCHET_NAME).write_text(json.dumps(ratchet()))
        git('add', '.')
        git('commit', '-q', '--no-gpg-sign', '-m', 'base')
        git('checkout', '-q', '-b', 'side')
        if side_raises:
            (root / pr.RATCHET_NAME).write_text(json.dumps(ratchet(warm=('lisp/a.el', 'lisp/b.el', 'lisp/c.el'))))
        else:
            (root / 'work').write_text('x')
            git('add', '.')
        git('commit', '-q', '--no-gpg-sign', '-am', 'side work')
        git('checkout', '-q', 'master')
        lowered = ratchet(warm=('lisp/a.el',))
        lowered['lowerings'] = [{'unit': 'census-warm', 'members': ['lisp/b.el'],
                                 'because': 'lisp/ was replaced by the GNU pin'}]
        (root / pr.RATCHET_NAME).write_text(json.dumps(lowered))
        git('commit', '-q', '--no-gpg-sign', '-am', 'master records a lowering')
        base = git('rev-parse', 'HEAD')
        git('checkout', '-q', '-b', 'lane')
        git('merge', '-q', '--no-ff', '--no-gpg-sign', '--no-commit', '-X', 'ours', 'side')
        (root / pr.RATCHET_NAME).write_text(json.dumps(lowered))
        git('add', pr.RATCHET_NAME)
        git('commit', '-q', '--no-gpg-sign', '-m', 'merge side')
        return root, base

    def verify(self, root, base):
        err = io.StringIO()
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err):
            status = pr.cmd_verify(type('A', (), {'root': str(root), 'base': base, 'candidate': True})())
        return status, err.getvalue()

    def test_a_branch_that_left_the_file_alone_merges(self):
        status, err = self.verify(*self.repo(side_raises=False))
        self.assertEqual(status, 0, err)

    def test_a_raise_the_merge_drops_is_refused(self):
        status, err = self.verify(*self.repo(side_raises=True))
        self.assertEqual(status, pr.EXIT_LOWER, err)
        self.assertIn('census-warm: 2 dropped (lisp/b.el, lisp/c.el)', err)


class Wiring(unittest.TestCase):
    def test_the_gate_runs_every_unit(self):
        self.assertEqual(pr.wiring(ROOT), [])

    def mutated(self, rel, edit):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = pathlib.Path(tmp.name)
        for r in pr.GATE_TESTS:
            (root / r).parent.mkdir(parents=True, exist_ok=True)
            (root / r).write_text((ROOT / r).read_text())
        before = (root / rel).read_text()
        (root / rel).write_text(edit(before))
        self.assertNotEqual((root / rel).read_text(), before, 'the edit did not apply')
        return pr.wiring(root)

    def test_a_dropped_shard_is_refused(self):
        problems = self.mutated('gnu-tests/runner/tests/parity_ratchet.rs',
                                lambda t: t.replace(f'"gnu-shard:{pr.GATE_SHARDS}/{pr.GATE_SHARDS}"',
                                                    f'"gnu-shard:{pr.GATE_SHARDS - 1}/{pr.GATE_SHARDS}"'))
        self.assertTrue(any('parity_ratchet.rs was edited' in p for p in problems), problems)

    def test_a_compiled_out_test_file_is_refused(self):
        for spelling in ('#![cfg(any())]\n', '#! [cfg(any())]\n', '#!/*x*/[cfg(any())]\n'):
            problems = self.mutated('gnu-tests/runner/tests/parity_ratchet.rs', lambda t: spelling + t)
            self.assertTrue(problems, spelling)

    def test_a_shard_without_its_test_attribute_is_refused(self):
        problems = self.mutated('crates/romacs-vm/tests/parity_ratchet.rs',
                                lambda t: t.replace('#[test]\nfn parity_ratchet_census_cold', 'fn parity_ratchet_census_cold'))
        self.assertTrue(problems)

    def test_a_missing_test_file_is_refused(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.assertEqual(len(pr.wiring(pathlib.Path(tmp.name))), len(pr.GATE_TESTS))


PIN_SHA = 'a31ba7aa12a7e4bbdda6ab9e19dab8338340f81f8100c33ac3843d10e176c8d8'
FNS_BASELINE = ROOT / 'gnu-tests' / 'baseline' / 'src' / 'fns-tests.el.json'
# Until the owner authorises re-recording the 565 GNU baselines with the
# pinned binary, run.py refuses every one of them (they were recorded by the
# distro /usr/bin/emacs-32.0.50), so nothing can be measured at all.
BASELINES_PINNED = json.loads(FNS_BASELINE.read_text())['gnu'].get('sha256') == PIN_SHA


def assert_distro_baseline_refused(case, refusal):
    case.assertEqual(refusal.status, pr.EXIT_NOT_MEASURED)
    case.assertIn('gnu-tests/baseline/src/fns-tests.el.json', str(refusal))
    case.assertIn(PIN_SHA, str(refusal))


class CensusPopulation(unittest.TestCase):
    def setUp(self):
        from unittest.mock import patch
        self.patch = patch
        self.directory = self.enterContext(tempfile.TemporaryDirectory(prefix='census-population-'))
        base = pathlib.Path(self.directory)
        self.root, self.gnu = base / 'root', base / 'gnu'
        for directory, lisp in ((self.root, 'gnu-lisp'), (self.gnu, 'lisp')):
            (directory / lisp).mkdir(parents=True)
        (self.root / 'tools').mkdir()
        import shutil
        shutil.copyfile(ROOT / 'tools/lisp_oracle.py', self.root / 'tools/lisp_oracle.py')
        for name in ('a.el', 'b.el'):
            for directory, lisp in ((self.root, 'gnu-lisp'), (self.gnu, 'lisp')):
                (directory / lisp / name).write_text('(provide \'fixture)\n')
        def git(*args):
            return subprocess.check_output(['git', '-C', str(self.gnu), *args], text=True).strip()
        git('init', '-q')
        git('add', '.')
        git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
            '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture')
        self.pin = git('rev-parse', 'HEAD')
        self.enterContext(patch.object(pr, 'GNU_PIN', self.pin))
        self.enterContext(patch.object(pr, 'GNU_SOURCE', self.gnu))
        self.support = 'gnu-lisp/generated.el'
        (self.root / self.support).write_text('(provide \'generated)\n')
        self.manifest = {'gnu_commit': self.pin,
                         'sources': {'fixture': {'paths': ['lisp/a.el'], 'recipe': 'fixture generator'}},
                         'files': [[self.support, pr.sha256_file(self.root / self.support), 'fixture']]}
        (self.root / 'LISP-PIN.json').write_text(json.dumps({
            'gnu_commit': self.pin, 'lisp_tree': git('rev-parse', 'HEAD:lisp'),
            'generated_support': 'LISP-GENERATED.json'}))
        self.write_manifest()

    def write_manifest(self):
        (self.root / 'LISP-GENERATED.json').write_text(json.dumps(self.manifest))

    def test_population_is_exact_pristine_pin_plus_validated_generated_support(self):
        pristine, support = pr.census_population(self.root)
        self.assertEqual(pristine, {'gnu-lisp/a.el', 'gnu-lisp/b.el'})
        self.assertEqual(support, {self.support})

    def test_missing_extra_modified_or_duplicate_population_is_refused(self):
        mutations = ('missing', 'extra', 'modified', 'support-bytes', 'support-pin', 'support-duplicate')
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                path = self.root / 'gnu-lisp/a.el'
                original = path.read_bytes()
                saved = copy.deepcopy(self.manifest)
                if mutation == 'missing':
                    path.unlink()
                elif mutation == 'extra':
                    (self.root / 'gnu-lisp/extra.el').write_text('nil')
                elif mutation == 'modified':
                    path.write_text('nil')
                elif mutation == 'support-bytes':
                    self.manifest['files'][0][1] = '0' * 64
                elif mutation == 'support-pin':
                    self.manifest['gnu_commit'] = '0' * 40
                else:
                    self.manifest['files'].append(self.manifest['files'][0])
                self.write_manifest()
                with self.assertRaises(pr.Refusal):
                    pr.census_population(self.root)
                path.write_bytes(original)
                (self.root / 'gnu-lisp/extra.el').unlink(missing_ok=True)
                self.manifest = saved
                self.write_manifest()

    def measure(self, oracle_edit=lambda rows: rows, row_edit=lambda row: row):
        from types import SimpleNamespace
        files = sorted((self.root / 'gnu-lisp').glob('*.el'))
        oracle = oracle_edit([{'path': str(p), 'form_count': 1, 'status': 'ok'} for p in files])
        self.executed = []
        def run_file(path, entry, *args):
            self.executed.append(path.relative_to(self.root).as_posix())
            return row_edit(dict(file=path.relative_to(self.root).as_posix(), forms_evaluated=1,
                                 total_forms=1, loaded=True, mode='cold'))
        lc = SimpleNamespace(ROOT=self.root, run_file=run_file, load_path=lambda root: [root / 'gnu-lisp'])
        load = pr.load_module
        def module(path, name):
            return lc if name == 'lisp_census' else load(path, name)
        run = pr.subprocess.run
        def command(argv, **kwargs):
            if argv[0] == '/fixture/oracle':
                return subprocess.CompletedProcess(argv, 0, json.dumps({'files': oracle}), '')
            return run(argv, **kwargs)
        with self.patch.object(pr, 'load_module', side_effect=module), \
                self.patch.object(pr, 'emacs', return_value='/fixture/oracle'), \
                self.patch.object(pr.subprocess, 'run', side_effect=command):
            return pr.measure_census(self.root, 'cold', '/bin/true', 1)

    def test_all_targets_execute_but_support_is_not_parity_credit(self):
        measured = self.measure()
        self.assertEqual(sorted(self.executed), ['gnu-lisp/a.el', 'gnu-lisp/b.el', self.support])
        self.assertEqual(measured['files_executed'], 3)
        self.assertEqual((measured['files_measured'], measured['files_reported']), (2, 2))
        self.assertEqual(measured['members']['census-cold'], ['lisp/a.el', 'lisp/b.el'])
        self.assertEqual(measured['verdicts']['lisp/a.el'], 'loaded')
        self.assertNotIn('gnu-lisp/a.el', measured['verdicts'])
        self.assertEqual(measured['generated_support'], {
            'total': 1, 'reported': 1, 'loaded': [self.support], 'unreported': []})

    def test_oracle_same_count_duplicates_reordering_missing_and_extra_are_refused(self):
        for edit in (lambda rows: [rows[0], rows[0], rows[2]], lambda rows: rows[::-1],
                     lambda rows: rows[:-1], lambda rows: rows + [rows[0]]):
            with self.subTest(edit=edit), self.assertRaises(pr.Refusal):
                self.measure(oracle_edit=edit)

    def test_vm_row_identity_substitution_is_refused(self):
        with self.assertRaises(pr.Refusal):
            self.measure(row_edit=lambda row: dict(row, file='lisp/a.el'))

    def test_partial_pristine_or_support_reports_cannot_pass(self):
        for missing in ('gnu-lisp/a.el', self.support):
            with self.subTest(missing=missing):
                measured = self.measure(row_edit=lambda row: dict(row, forms_evaluated=None, loaded=False)
                                        if row['file'] == missing else row)
                data = ratchet(cold=('lisp/a.el', 'lisp/b.el'))
                data['census']['cold'].update(gnu_target_total=2, shared_total=2, files_measured=2)
                status, text = run_report(data, 'census-cold', measured)
                self.assertEqual(status, pr.EXIT_NOT_MEASURED, text)
                self.assertNotIn('PASS', text)


class NothingMeasured(unittest.TestCase):
    """Zero tests executed is RED, never PASS."""

    def test_missing_binary_is_not_measured(self):
        with self.assertRaises(pr.Refusal) as e:
            pr.measure_gnu(ROOT, ['src/fns-tests.el'], '/nonexistent/romacs-gnu-ert', 2)
        self.assertEqual(e.exception.status, pr.EXIT_NOT_MEASURED)

    def test_an_adapter_that_runs_nothing_is_not_measured(self):
        with self.assertRaises(pr.Refusal) as e:
            pr.measure_gnu(ROOT, ['src/fns-tests.el'], '/bin/true', 2)
        if not BASELINES_PINNED:
            return assert_distro_baseline_refused(self, e.exception)
        self.assertEqual(e.exception.status, pr.EXIT_NOT_MEASURED)
        self.assertIn('zero tests executed', str(e.exception))

    def test_no_files_is_not_measured(self):
        with self.assertRaises(pr.Refusal):
            pr.measure_gnu(ROOT, [], '/bin/true', 2)

    def test_a_census_binary_that_reports_nothing_is_not_measured(self):
        with self.assertRaises(pr.Refusal) as e:
            pr.measure_census(ROOT, 'cold', '/bin/true', 8)
        self.assertEqual(e.exception.status, pr.EXIT_NOT_MEASURED)

    def test_refusal_exits_nonzero_and_never_prints_pass(self):
        p = subprocess.run([sys.executable, str(pathlib.Path(pr.__file__)), 'check', '--unit',
                            'gnu-files:src/fns-tests.el', '--gnu-ert', '/bin/true'],
                           capture_output=True, text=True, cwd=ROOT)
        self.assertEqual(p.returncode, pr.EXIT_NOT_MEASURED, p.stdout + p.stderr)
        self.assertNotIn('PASS', p.stdout)

    def test_sealed_evidence_must_match_measured_executable(self):
        from unittest.mock import patch
        sys.path.insert(0, str(ROOT / 'gnu-tests/tools'))
        import vacuity_binding
        binary = '/bin/true'
        def measured(command, **kwargs):
            self.assertEqual(command[command.index('--harness') + 1], 'mini')
            self.assertEqual(command[command.index('--bootstrap') + 1], 'ert')
            output = pathlib.Path(command[command.index('--out') + 1])
            output.mkdir()
            (output / 'summary.json').write_text('{}')
            (output / 'results.jsonl').write_text(json.dumps(dict(
                file='src/fns-tests.el', test='probe', verdict='passed', romacs={})))
            return subprocess.CompletedProcess(command, 0, '', '')
        evidence = {'src/fns-tests.el::probe': {'class': 'GENUINE', 'method': 'sealed-subject-v1'}}
        with patch.object(pr.subprocess, 'run', side_effect=measured), \
                patch.object(pr, 'load_evidence', return_value=evidence):
            for digest in ('0' * 64, pr.sha256_file(binary)):
                manifest = {'binaries': {'romacs': {'file': {'sha256': digest}}}}
                with patch.object(vacuity_binding, 'verify_receipt', return_value=({}, manifest, {})):
                    if digest == pr.sha256_file(binary):
                        self.assertEqual(pr.measure_gnu(ROOT, ['src/fns-tests.el'], binary, 1)['binary_sha256'], digest)
                    else:
                        with self.assertRaisesRegex(pr.Refusal, 'subject evidence binary differs from measured romacs binary'):
                            pr.measure_gnu(ROOT, ['src/fns-tests.el'], binary, 1)

    def test_the_real_adapter_is_measured(self):
        binary = os.environ.get('ROMACS_GNU_ERT')
        if not binary:
            raise AssertionError('ROMACS_GNU_ERT must name the built adapter')
        if not BASELINES_PINNED:
            with self.assertRaises(pr.Refusal) as e:
                pr.measure_gnu(ROOT, ['src/fns-tests.el'], binary, 2)
            return assert_distro_baseline_refused(self, e.exception)
        m = pr.measure_gnu(ROOT, ['src/fns-tests.el'], binary, 2)
        self.assertTrue(m['classes'] is not None and all(
            c['class'] in pr.MEMBER_CLASSES for c in m['classes'].values()))
        self.assertGreater(m['romacs_reported'], 0)
        self.assertEqual(m['binary_sha256'], pr.sha256_file(binary))
        self.assertEqual(set(m['members']), {'src/fns-tests.el'})


class Shards(unittest.TestCase):
    def test_shards_partition_the_suite(self):
        files = pr.suite_files(ROOT)
        parts = [pr.shard(files, f'{k}/{pr.GATE_SHARDS}') for k in range(1, pr.GATE_SHARDS + 1)]
        self.assertEqual(sorted(f for p in parts for f in p), sorted(files))
        self.assertTrue(all(parts))
        self.assertLessEqual(max(map(len, parts)), 8)

    def test_the_ratchet_covers_exactly_the_suite(self):
        data = pr.load_ratchet(ROOT)
        self.assertEqual(sorted(data['gnu_suite']['files']), sorted(pr.suite_files(ROOT)))


if __name__ == '__main__':
    unittest.main()
