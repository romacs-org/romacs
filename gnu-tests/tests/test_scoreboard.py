"""Real ERT CLI and fidelity contracts; native execution lives in test_ert_report."""
import contextlib
import io
import pathlib
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1]))
import run


class Scoreboard(unittest.TestCase):
    def test_real_cli_dispatch(self):
        with patch.object(run, 'cmd_run', return_value=19) as command:
            try:
                result = run.main(['run', '--harness', 'real', '--engine', 'gnu', 'lisp/abbrev-tests.el'])
            except SystemExit:
                result = None
            self.assertEqual(result, 19, 'REAL-CLI: real harness must reach runner')
            self.assertEqual(command.call_args.args[0].harness, 'real')

    def test_fidelity_cli_dispatch(self):
        with patch.object(run, 'cmd_run', return_value=23) as command:
            try:
                result = run.main(['run', '--harness-fidelity', 'lisp/abbrev-tests.el'])
            except SystemExit:
                result = None
            self.assertEqual(result, 23, 'FIDELITY-CLI: fidelity flag must reach runner')
            self.assertTrue(command.call_args.args[0].harness_fidelity)

    def compare(self, real, mini):
        self.assertTrue(hasattr(run, 'harness_defects'), 'FIDELITY-DIFF: every identity and verdict must be compared')
        return run.harness_defects(real, mini)

    def test_fidelity_checks_every_identity_and_verdict(self):
        real = {'a': {'outcome': 'passed'}, 'b': {'outcome': 'failed'}, 'c': {'outcome': 'skipped'}}
        mini = {'a': {'outcome': 'passed'}, 'b': {'outcome': 'passed'}, 'extra': {'outcome': 'passed'}}
        self.assertEqual([r['test'] for r in self.compare(real, mini)], ['b', 'c', 'extra'],
                         'FIDELITY-DIFF: missing, extra and mismatched verdicts are harness defects')
        self.assertEqual(self.compare(real, real), [])

    def test_runtime_inventory_must_match_reference(self):
        self.assertTrue(hasattr(run, 'scoreboard_rows'), 'REAL-INVENTORY: missing reference tests cannot disappear')
        real = {'a': {'outcome': 'passed', 'as_expected': True},
                'b': {'outcome': 'passed', 'as_expected': True}}
        rows = run.scoreboard_rows('f', real, {'a': real['a']}, 'romacs', {})
        self.assertEqual([row['test'] for row in rows], ['a', 'b'],
                         'REAL-INVENTORY: missing reference tests cannot disappear')
        self.assertEqual(rows[1]['verdict'], 'failed', 'REAL-INVENTORY: missing test is red')

    def test_fidelity_uses_ert_failure_class(self):
        self.assertEqual(self.compare({'error': {'outcome': 'failed'}},
                                      {'error': {'outcome': 'errored'}}), [],
                         'FIDELITY-CLASS: errors are ERT failures, not harness defects')

    def test_scoreboard_process_failure_is_red(self):
        import argparse
        import json
        import tempfile
        import ert_report
        sys.path.insert(0, str(run.ROOT / 'scripts'))
        import gnu_oracle
        with tempfile.TemporaryDirectory() as tmp:
            args = argparse.Namespace(all=False, files=['lisp/emacs-lisp/ring-tests.el'],
                                      jobs=1, out=str(pathlib.Path(tmp) / 'report'),
                                      harness_fidelity=False, engine='gnu', bin=None, setup_timeout=10)
            with (patch.object(run, 'verify_pin', return_value={}),
                  patch.object(run, 'normalize', side_effect=lambda name: name),
                  patch.object(gnu_oracle, 'require', return_value={'path': '/pinned/emacs'}),
                  patch.object(run, 'load_ledger', return_value={}),
                  patch.object(run, 'git_tree_id', return_value='tree'),
                  patch.object(run.subprocess, 'run', return_value=argparse.Namespace(stdout='tree')),
                  patch.object(run, 'ScratchTree', return_value=contextlib.nullcontext()),
                  patch.object(ert_report, 'run_native', side_effect=ert_report.ReportError('truncated ERT summary')),
                  contextlib.redirect_stdout(io.StringIO())):
                status = run.cmd_scoreboard(args)
            self.assertEqual(status, 4, 'REAL-PROCESS: incomplete process must make scoreboard red')
            summary = json.loads((pathlib.Path(args.out) / 'summary.json').read_text())
            self.assertEqual(summary['completed_files'], 0, 'REAL-PROCESS: partial run cannot be complete')
            self.assertEqual(summary['infrastructure_errors'],
                             {'lisp/emacs-lisp/ring-tests.el': 'truncated ERT summary'})

    def test_vacuous_pass_is_red_in_scoreboard_and_fidelity(self):
        result = {'outcome': 'passed', 'as_expected': True,
                  'vacuous_void_function': ['missing-implementation']}
        rows = run.scoreboard_rows('f', {'a': result}, {'a': result}, 'gnu', {})
        self.assertEqual(rows[0]['verdict'], 'failed', 'VACUOUS-SCORE: caught void-function is not parity')
        self.assertEqual(len(self.compare({'a': result}, {'a': result})), 1,
                         'VACUOUS-FIDELITY: agreeing vacuous passes must be red')

    def test_empty_fidelity_is_not_green(self):
        self.assertTrue(hasattr(run, 'harness_defects'), 'FIDELITY-EMPTY: empty fidelity cannot be green')
        with self.assertRaisesRegex(run.Refusal, 'zero tests', msg='FIDELITY-EMPTY: empty fidelity cannot be green'):
            run.harness_defects({}, {})


if __name__ == '__main__':
    unittest.main()
