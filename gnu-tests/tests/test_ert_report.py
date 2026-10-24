"""Native ERT reporter contracts; runnable without the Rust runner."""
import inspect
import json
import os
import pathlib
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import ert_report


def event(kind, **fields):
    return 'ERT-SCOREBOARD ' + json.dumps(dict(event=kind, **fields)) + '\n'


def report(rows=None):
    rows = rows if rows is not None else [('a', 'passed', True)]
    skipped = sum(outcome == 'skipped' for _, outcome, _ in rows)
    expected = sum(ok and outcome != 'skipped' for _, outcome, ok in rows)
    unexpected = len(rows) - expected - skipped
    return (event('registered', names=[name for name, _, _ in rows])
            + ''.join(event('result', test=name, outcome=outcome, as_expected=ok)
                      for name, outcome, ok in rows)
            + f'Ran {len(rows)} tests, {expected} results as expected, {unexpected} unexpected'
              + (f', {skipped} skipped' if skipped else '') + ' (date, 0.001 sec)\n'
            + event('done'))


class Parser(unittest.TestCase):
    def rejects(self, text, rc=0):
        with self.assertRaises(ValueError, msg=f'{inspect.stack()[1].function}: invalid ERT report accepted') as caught:
            ert_report.parse_report(text, rc)
        self.assertEqual(getattr(caught.exception, 'output'), text)

    def test_valid_report(self):
        value = ert_report.parse_report(report(), 0)
        self.assertEqual(value['results'], {'a': {'outcome': 'passed', 'as_expected': True}})
        self.assertEqual(value['summary'], dict(passed=1, failed=0, skipped=0, total=1))
        self.assertEqual(value['registered'], 1)
        self.assertIs(value['complete'], True)

    def test_expected_failure_and_skip_are_raw_outcomes(self):
        value = ert_report.parse_report(report([('xfail', 'failed', True), ('skip', 'skipped', False)]), 0)
        self.assertEqual(value['summary'], dict(passed=0, failed=1, skipped=1, total=2),
                         'ERT-RAW-OUTCOMES: expected failure must not become pass')
        self.assertTrue(value['results']['xfail']['as_expected'])

    def test_completed_unexpected_failure_is_valid(self):
        self.assertEqual(ert_report.parse_report(report([('bad', 'failed', False)]), 1)['summary']['failed'], 1)

    def test_zero_registered_is_not_success(self):
        self.rejects(report([]))

    def test_death_even_with_complete_output(self):
        self.rejects(report(), -9)

    def test_exit_status_must_match_unexpected_results(self):
        self.rejects(report(), 1)
        self.rejects(report([('bad', 'failed', False)]), 0)
        self.rejects(report(), 2)

    def test_absent_truncated_and_garbled_reports(self):
        for text in ['', 'random chatter', report().split('Ran ')[0],
                     report().replace(event('done'), ''),
                     report().replace(event('done'), 'ERT-SCOREBOARD {oops}\n'),
                     report().replace('1 results as expected', 'garbage results as expected')]:
            with self.subTest(text=text):
                self.rejects(text)

    def test_registered_identity_mismatch(self):
        self.rejects(report().replace('"test": "a"', '"test": "b"'))

    def test_duplicate_rows_and_inventory(self):
        self.rejects(report() + event('result', test='a', outcome='passed', as_expected=True))
        self.rejects(report([('a', 'passed', True), ('a', 'passed', True)]))
        self.rejects(report().replace(event('done'), event('done') * 2))

    def test_missing_row_and_summary_mismatch(self):
        self.rejects(report().replace(event('result', test='a', outcome='passed', as_expected=True), ''))
        self.rejects(report().replace('Ran 1 tests', 'Ran 2 tests'))
        self.rejects(report().replace('1 results as expected, 0 unexpected', '0 results as expected, 1 unexpected'), 1)

    def test_invalid_row_types_and_unknown_outcome(self):
        for old, new in [('"passed"', '"unknown"'), ('true', '1')]:
            self.rejects(report().replace(old, new))
        self.rejects(report().replace('"names": ["a"]', '"names": "a"'))

    def test_protocol_order_and_duplicate_summary(self):
        self.rejects(event('done') + report())
        self.rejects(report() + 'Ran 1 tests, 1 results as expected, 0 unexpected (date, 0.1 sec)\n')
        self.rejects(report().replace('Ran 1', 'Aborted: Ran 1'))


class Native(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.ert = pathlib.Path(__file__).resolve().parents[2] / 'lisp/emacs-lisp/ert.el'
        sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / 'scripts'))
        import gnu_oracle
        binary = os.environ.get('ROMACS_GNU_ORACLE') or os.environ.get('ROMACS_FORCE_ORACLE_PATH')
        self.binary = gnu_oracle.require(binary)['path']

    def run_source(self, source, timeout: float = 15):
        target = self.root / 'fixture.el'
        target.write_text(source)
        return ert_report.run_native(self.binary, target, None, self.ert, [], self.root, timeout)

    def test_genuine_batch_summary_with_expected_failure_and_skip(self):
        value = self.run_source('''
(ert-deftest a-pass () (should t))
(ert-deftest b-xfail () :expected-result :failed (should nil))
(ert-deftest c-skip () (ert-skip "reason"))
(ert-deftest d-fail () (error "failure"))
''')
        self.assertEqual(value['summary'], dict(passed=1, failed=2, skipped=1, total=4))
        self.assertIn('Ran 4 tests, 2 results as expected, 1 unexpected, 1 skipped', value['output'])
        self.assertTrue(value['results']['b-xfail']['as_expected'])
        self.assertEqual(value['returncode'], 1)
        self.assertEqual(value['status'], 'complete')

    def test_zero_load_error_death_and_timeout_rejected(self):
        for source in ['', '(error "load broke")', '(kill-emacs 0)',
                       '(ert-deftest death () (kill-emacs 0))',
                       '(ert-deftest death () (signal-process (emacs-pid) 9))']:
            with self.subTest(source=source), self.assertRaises(ValueError):
                self.run_source(source)
        with self.assertRaises(ValueError) as caught:
            self.run_source('(ert-deftest hang () (while t (sleep-for 1)))', timeout=5)
        self.assertIn('ERT-SCOREBOARD', caught.exception.output)

    def test_caught_void_function_is_not_parity(self):
        value = self.run_source('''
(ert-deftest vacuous () (should-error (scoreboard-missing-function)))
(ert-deftest intentional () (should-error (scoreboard-missing-function) :type 'void-function))
''')
        self.assertEqual(value['results']['vacuous'].get('vacuous_void_function'),
                         ['scoreboard-missing-function'],
                         'VACUOUS-PASS: caught missing implementation must be exposed')
        self.assertNotIn('vacuous_void_function', value['results']['intentional'])

    def test_rx_redefine_builtin_native_control(self):
        import run
        from unittest.mock import patch
        source = (pathlib.Path(__file__).resolve().parents[1] /
                  'test/lisp/emacs-lisp/rx-tests.el').read_text()
        real = self.run_source(source)
        with patch.object(run, 'RUN_TESTS', self.root):
            plan = run.extract('fixture.el', self.root, self.binary)
            mini = run.gnu_run('mini', 'fixture.el', self.root, 30, plan, self.binary)
        self.assertEqual(real['results']['rx-redefine-builtin']['outcome'], 'passed',
                         'RX-CONTROL: genuine ERT must run the named case')
        self.assertEqual(mini['results']['rx-redefine-builtin']['outcome'], 'passed',
                         'RX-CONTROL: GNU mini control must not invent a VM-only defect')
        self.assertEqual(run.harness_defects(real['results'], mini['results']), [])

    def test_pinned_ert_and_mini_are_not_silently_ignored(self):
        (self.root / 'fixture.el').write_text('(ert-deftest fixture () (should t))')
        with self.assertRaises(ValueError):
            ert_report.run_native(self.binary, self.root / 'fixture.el', None,
                                  self.root / 'absent.el', [], self.root, 5)
        with self.assertRaises(ValueError):
            ert_report.run_native(self.binary, self.root / 'fixture.el', self.ert,
                                  self.ert, [], self.root, 5)


if __name__ == '__main__':
    unittest.main()
