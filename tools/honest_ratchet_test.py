#!/usr/bin/env python3
"""The honest GNU-suite ratchet's own tests: member classes, the vacuous
budget, counting only files that loaded, and the labelled real-ERT number.
Run in the gate with tools/parity_ratchet_test.py (which imports this file's
classes). Each test's docstring names the defect it must catch.
"""
import collections
import contextlib
import copy
import io
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import parity_ratchet as pr  # noqa: E402

RX = 'lisp/emacs-lisp/rx-tests.el'
SUBR = 'lisp/subr-tests.el'
WHY = 'owner restatement: these members pass only because their subject is missing'


def ev(cls, method='mutation', gnu_caught=()):
    return {'class': cls, 'method': method, 'why': f'fixture {cls} by {method}', 'gnu_caught': list(gnu_caught)}


def fa(loaded=True, failed=(), vacuous=()):
    return {'file_loaded_clean': loaded, 'failed_forms': list(failed), 'vacuous_catch': list(vacuous)}


def files_of(keys):
    return sorted({k.split('::', 1)[0] for k in keys} | set(pr.CHARACTERISED) | {'lisp/other-tests.el', RX})


def classified(rows):
    members, classes = collections.defaultdict(list), {}
    for key, (evidence, facts) in rows.items():
        f, t = key.split('::', 1)
        members[f].append(t)
        classes[key] = pr.classify_member(evidence, pr.member_facts(facts))
    return members, classes


CENSUS = {m: {'gnu_parity_loaded': 1, 'gnu_target_total': 1596, 'shared_total': 1590, 'files_measured': 1811,
              'chain': None, 'loaded': ['lisp/a.el']} for m in pr.CENSUS_MODES}


def honest(rows, real_passed=0, files=None):
    """A schema-2 ratchet recorded from ROWS = {file::test: (evidence, facts)}."""
    members, classes = classified(rows)
    files = files or files_of(rows)
    total = sum(len(v) for v in members.values())
    real = {'harness': pr.REAL_LABEL, 'is_gnu_ert': True, 'passed': real_passed, 'of': total,
            'passing': {}, 'files_aborted': 0}
    data = {'schema': 2, 'gnu_suite': pr.honest_suite(files, {f: 10 for f in files}, members, classes, real),
            'census': copy.deepcopy(CENSUS), 'lowerings': []}
    assert pr.consistency(data) == [], pr.consistency(data)
    return data


def measured(rows, files):
    """A GNU measurement in which exactly ROWS passed."""
    members, classes = classified(rows)
    return {'kind': 'gnu', 'files': files, 'gnu_tests': 10 * len(files), 'romacs_reported': 5,
            'tests_per_file': {f: 10 for f in files}, 'members': {f: sorted(members.get(f, [])) for f in files},
            'classes': classes, 'verdicts': {f'{f}::{t}': 'failed (honest)' for f in files for t in ('x',)},
            'seconds': 0.1}


def report(data, m):
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        status = pr.report('gnu-files:x', data, m)
    return status, buf.getvalue()


GENUINE = (ev('GENUINE'), fa())
RX_VACUOUS = (ev('VACUOUS', 'hand'), fa())
# Recorded from a run in which absent machinery satisfied its should-errors.
RX_VOID = (ev('VACUOUS', 'void-catch'), fa(vacuous=['(void-variable sequence)']))


class HonestRatchet(unittest.TestCase):

    def test_converting_a_vacuous_pass_into_an_honest_failure_is_progress(self):
        """Mutant progress-is-loss: a budget member's loss is routed to FAIL."""
        data = honest({f'{RX}::rx-and': GENUINE, f'{RX}::rx-redefine-builtin': RX_VACUOUS})
        status, out = report(data, measured({f'{RX}::rx-and': GENUINE}, data['gnu_suite']['files']))
        self.assertEqual(status, pr.EXIT_HIGHER,
                         'converting a vacuous pass into an honest failure read as a floor loss:\n' + out)
        self.assertIn(f'PROGRESS budget:{RX}: vacuous budget 1 -> 0', out)
        self.assertIn('rx-redefine-builtin', out)
        self.assertNotIn('parity-ratchet: FAIL ', out)

    def test_a_genuine_regression_still_fails_the_gate(self):
        """Mutant genuine-loss-is-progress: a protected member's loss is routed to PROGRESS."""
        # rx-and is GENUINE but its file failed a top-level form, as 187 of the
        # 198 genuine members' files do today: not counted, still protected.
        unloaded = (ev('GENUINE'), fa(loaded=False, failed=[1]))
        data = honest({f'{RX}::rx-and': unloaded, f'{RX}::rx-redefine-builtin': RX_VACUOUS})
        self.assertEqual(data['gnu_suite']['honest']['counted'], 0)
        # The vacuous member also failed honestly in the same run: progress
        # elsewhere in the unit must not pay for the genuine loss.
        status, out = report(data, measured({}, data['gnu_suite']['files']))
        self.assertEqual(status, pr.EXIT_LOWER, 'a GENUINE floor member was lost and the gate did not fail:\n' + out)
        self.assertIn(f'FAIL gnu:{RX}', out)
        self.assertIn('- rx-and: now', out)
        self.assertIn('class GENUINE', out, 'a lost floor member must name its recorded class')

    def test_a_vacuous_pass_cannot_be_counted_as_genuine(self):
        """Mutant vacuous-counted: classify_member ignores the observed void catch."""
        c = pr.classify_member(ev('GENUINE'), pr.member_facts(fa(vacuous=['(void-function read)'])))
        self.assertFalse(c['counted'], 'a vacuous pass was counted as genuine')
        self.assertEqual(c['class'], 'VACUOUS')
        for cls in pr.BUDGET_CLASSES:
            c = pr.classify_member(ev(cls, 'hand'), pr.member_facts(fa()))
            self.assertFalse(c['counted'], f'a {cls} pass was counted as genuine')
        data = honest({f'{RX}::rx-and': GENUINE, f'{RX}::rx-redefine-builtin': RX_VACUOUS})
        self.assertEqual(data['gnu_suite']['honest']['counted'], 1, 'a vacuous pass was counted as genuine')
        self.assertEqual(data['gnu_suite']['vacuous_budget'], {RX: ['rx-redefine-builtin']})

    def test_the_vacuous_budget_cannot_increase(self):
        """Mutant budget-may-grow: a new budget member is accepted."""
        data = honest({f'{RX}::rx-and': GENUINE, f'{RX}::rx-redefine-builtin': RX_VACUOUS})
        files = data['gnu_suite']['files']
        grown = {f'{RX}::rx-and': GENUINE, f'{RX}::rx-redefine-builtin': RX_VACUOUS,
                 f'{RX}::rx-new': (ev('VACUOUS', 'hand'), fa())}
        status, out = report(data, measured(grown, files))
        self.assertEqual(status, pr.EXIT_LOWER, 'the vacuous budget grew and the gate did not fail:\n' + out)
        self.assertIn(f'BUDGET-INCREASE budget:{RX}', out)
        new = honest(grown, files=files)
        with self.assertRaises(pr.Refusal, msg='the vacuous budget grew and record accepted it') as cm:
            pr.apply_lowerings(data, new, {}, {})
        self.assertIn('vacuous budget', str(cm.exception))
        problems = pr.lowering_problems(data, new, 'step')
        self.assertTrue(any('vacuous budget' in p for p in problems),
                        f'the vacuous budget grew and verify accepted it: {problems}')

    def test_a_member_whose_file_failed_to_load_is_not_counted(self):
        """Mutant unloaded-counted: counted ignores whether the file loaded."""
        c = pr.classify_member(ev('GENUINE'), pr.member_facts(fa(loaded=False, failed=[1])))
        self.assertFalse(c['counted'], 'a member whose file failed a top-level form was counted')
        self.assertEqual((c['class'], c['failed_forms']), ('GENUINE', [1]))
        data = honest({f'{SUBR}::s-loaded': GENUINE, f'{RX}::rx-and': (ev('GENUINE'), fa(loaded=False, failed=[1]))})
        self.assertEqual(data['gnu_suite']['honest']['counted'], 1,
                         'a member whose file failed a top-level form was counted')
        # A counted member whose file stops loading is a lost count, not a pass.
        now = {f'{SUBR}::s-loaded': (ev('GENUINE'), fa(loaded=False, failed=[3])),
               f'{RX}::rx-and': (ev('GENUINE'), fa(loaded=False, failed=[1]))}
        status, out = report(data, measured(now, data['gnu_suite']['files']))
        self.assertEqual(status, pr.EXIT_LOWER, 'a counted member whose file stopped loading still passed:\n' + out)
        self.assertIn(f'FAIL counted:{SUBR}', out)

    def test_a_vacuous_member_that_now_passes_for_a_real_reason_improves_both_numbers(self):
        data = honest({f'{RX}::rx-and': GENUINE, f'{RX}::rx-redefine-builtin': RX_VOID})
        self.assertEqual(data['gnu_suite']['vacuous_budget'], {RX: ['rx-redefine-builtin']})
        # The void catch the verdict rested on is gone: the evidence is stale.
        status, out = report(data, measured({f'{RX}::rx-and': GENUINE,
                                             f'{RX}::rx-redefine-builtin': (ev('VACUOUS', 'void-catch'), fa())},
                                            data['gnu_suite']['files']))
        self.assertEqual(status, pr.EXIT_HIGHER, out)
        self.assertIn(f'PROGRESS budget:{RX}', out)
        self.assertIn(f'HIGHER gnu:{RX}', out)
        c = pr.classify_member(ev('VACUOUS', 'void-catch'), pr.member_facts(fa()))
        self.assertEqual((c['class'], c['counted']), ('UNPROBEABLE', False))

    def test_a_member_with_no_probe_evidence_is_protected_but_not_counted(self):
        c = pr.classify_member(None, pr.member_facts(fa()))
        self.assertEqual((c['class'], c['counted']), ('UNPROBEABLE', False))

    def test_an_unchanged_honest_measurement_passes(self):
        rows = {f'{RX}::rx-and': GENUINE, f'{RX}::rx-redefine-builtin': (ev('VACUOUS', 'hand'), fa())}
        data = honest(rows)
        status, out = report(data, measured(rows, data['gnu_suite']['files']))
        self.assertEqual(status, pr.EXIT_PASS, out)
        self.assertIn('parity-ratchet: PASS gnu-files:x: measured 1 floor 1', out)
        self.assertIn('vacuous-budget 1/1', out)
        self.assertIn('real-ERT 0/2', out)

    def test_the_restatement_is_one_lowering_act(self):
        old = {'schema': 1, 'gnu_suite': {}, 'census': copy.deepcopy(CENSUS), 'lowerings': []}
        files = files_of([f'{RX}::x'])
        old['gnu_suite'] = {'files': files, 'file_count': len(files), 'tests_per_file': {f: 10 for f in files},
                            'gnu_tests': 10 * len(files), 'passed': 2,
                            'characterised': {f: 0 for f in pr.CHARACTERISED},
                            'passing': {RX: ['rx-and', 'rx-redefine-builtin']}}
        self.assertEqual(pr.consistency(old), [])
        new = honest({f'{RX}::rx-and': GENUINE, f'{RX}::rx-redefine-builtin': RX_VACUOUS}, files=files)
        with self.assertRaises(pr.Refusal, msg='a floor member moved into the vacuous budget without --lower'):
            pr.apply_lowerings(old, copy.deepcopy(new), {}, {})
        pr.apply_lowerings(old, new, {f'gnu:{RX}': WHY}, {})
        self.assertEqual(new['lowerings'][-1]['members'], ['rx-redefine-builtin'])
        self.assertEqual(pr.lowering_problems(old, new, 'restatement'), [])

    def test_classes_are_derived_not_hand_written(self):
        rows = {f'{RX}::rx-and': GENUINE, f'{RX}::rx-redefine-builtin': (ev('VACUOUS', 'hand'), fa())}
        data = honest(rows)
        evidence = {k: e for k, (e, _) in rows.items()}
        self.assertEqual(pr.regeneration_problems(data, evidence), [])
        forged = copy.deepcopy(data)
        forged['gnu_suite']['classes'][f'{RX}::rx-redefine-builtin']['class'] = 'GENUINE'
        self.assertTrue(pr.regeneration_problems(forged, evidence),
                        'a hand-edited member class was accepted as derived')
        self.assertTrue(pr.regeneration_problems(data, dict(evidence, **{f'{RX}::rx-and': ev('VACUOUS', 'hand')})),
                        'classes that no longer follow from the evidence were accepted')

    def test_real_ert_is_its_own_labelled_field(self):
        data = honest({f'{RX}::rx-and': GENUINE})
        g = data['gnu_suite']
        self.assertEqual(g['reduced_harness'], pr.REDUCED_LABEL)
        self.assertIn('NOT GNU ert.el', g['reduced_harness'])
        for forge in ({'harness': 'romacs-mini-ert'}, {'is_gnu_ert': False}, {'passed': 2}, {'of': 7}):
            bad = copy.deepcopy(data)
            bad['gnu_suite']['real_ert'].update(forge)
            self.assertTrue(pr.consistency(bad), f'a mislabelled real-ERT field was accepted: {forge}')
        bad = copy.deepcopy(data)
        del bad['gnu_suite']['real_ert']
        self.assertTrue(pr.consistency(bad), 'a schema-2 ratchet without its real-ERT number was accepted')

    def test_adapted_ert_cannot_be_labelled_unmodified(self):
        data = honest({f'{RX}::rx-and': GENUINE})
        real = data['gnu_suite']['real_ert']
        real.update(harness=pr.BATCH_REAL_LABEL, is_unmodified_gnu_ert=False,
                    ert_adapter=dict(name='romacs-batch', automatic_window_isolation=False))
        self.assertEqual(pr.consistency(data), [])
        for forged in [dict(harness=pr.REAL_LABEL), dict(is_unmodified_gnu_ert=True),
                       dict(ert_adapter=None), dict(ert_adapter='romacs-batch'),
                       dict(ert_adapter=dict(name='romacs-batch', automatic_window_isolation=True))]:
            bad = copy.deepcopy(data)
            bad['gnu_suite']['real_ert'].update(forged)
            self.assertTrue(pr.consistency(bad), f'adapted ERT evidence mislabelled: {forged}')

    def test_real_measurement_requires_and_reports_the_batch_adapter(self):
        from unittest.mock import patch
        import json
        summary = dict(is_gnu_ert=True, engine='romacs', bootstrap_regime='gnu-ert',
                       is_unmodified_gnu_ert=False,
                       ert_adapter=dict(name='romacs-batch', automatic_window_isolation=False))

        def measured(command, **kwargs):
            self.assertEqual(command[command.index('--ert-adapter') + 1], 'romacs-batch')
            self.assertEqual(command[command.index('--bootstrap') + 1], 'ert')
            out = pathlib.Path(command[command.index('--out') + 1])
            out.mkdir()
            (out / 'summary.json').write_text(json.dumps(summary))
            (out / 'results.jsonl').write_text(json.dumps(dict(file=RX, test='rx-and', verdict='passed')))
            return pr.subprocess.CompletedProcess(command, 0, '', '')

        with patch.dict(pr.os.environ, ROMACS_GNU_ORACLE='/fixture/gnu'), \
                patch.object(pr.subprocess, 'run', side_effect=measured):
            result = pr.measure_real_ert(pathlib.Path('.'), {RX: ['rx-and']}, '/fixture/romacs', 1)
            self.assertEqual(result['harness'], pr.BATCH_REAL_LABEL)
            self.assertFalse(result['is_unmodified_gnu_ert'])
            self.assertEqual(result['passed'], 1)
            summary['is_unmodified_gnu_ert'] = True
            with self.assertRaises(pr.Refusal):
                pr.measure_real_ert(pathlib.Path('.'), {RX: ['rx-and']}, '/fixture/romacs', 1)

    def test_the_restatement_report_shows_closing_arithmetic_and_every_moved_member(self):
        import restatement_report as rr
        files = files_of([f'{RX}::x'])
        old = {'schema': 1, 'census': copy.deepcopy(CENSUS), 'lowerings': [],
               'gnu_suite': {'passed': 3, 'passing': {RX: ['rx-and', 'rx-gone', 'rx-redefine-builtin']}}}
        new = honest({f'{RX}::rx-and': GENUINE, f'{RX}::rx-redefine-builtin': RX_VACUOUS,
                      f'{SUBR}::s-new': (None, fa())}, files=files)
        text = rr.render(old, new, {f'{RX}::rx-gone': ev('VACUOUS', 'void-catch')}, 'CMD')
        self.assertNotIn('DOES NOT CLOSE', text)
        for needle in ('| recorded floor today', '| 3 |', '**0 of 3**', '`rx-gone`', '`rx-redefine-builtin`',
                       '`s-new`', 'CMD', pr.REAL_LABEL, pr.REDUCED_LABEL):
            self.assertIn(needle, text)


if __name__ == '__main__':
    unittest.main()
