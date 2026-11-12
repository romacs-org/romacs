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
    def test_real_lisp_uses_declared_support_verifier(self):
        import subprocess
        with patch.object(run.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as command:
            run.verify_real_lisp(run.PIN_COMMIT)
        self.assertEqual(command.call_args.args[0], [
            sys.executable, str(run.ROOT / 'tools/lib/lisp_oracle.py'), 'check',
            '--root', str(run.ROOT), '--gnu', str(run.GNU_MIRROR), '--pin', run.PIN_COMMIT])

    def test_real_lisp_validation_fails_closed(self):
        import subprocess
        for failure in [OSError('missing verifier'),
                        subprocess.CompletedProcess([], 1, 'MODIFIED pristine', ''),
                        subprocess.CompletedProcess([], 1, '', 'ABSENT SUPPORT'),
                        subprocess.CompletedProcess([], 1, 'UNDECLARED file', '')]:
            with self.subTest(failure=failure):
                kwargs = {'side_effect': failure} if isinstance(failure, OSError) else {'return_value': failure}
                with patch.object(run.subprocess, 'run', **kwargs), self.assertRaises(run.Refusal) as caught:
                    run.verify_real_lisp(run.PIN_COMMIT)
                self.assertEqual(caught.exception.status, 3)

    def test_ert_bootstrap_is_explicit_and_separate(self):
        from types import SimpleNamespace
        self.assertIs(run.source_bootstrap_chain(SimpleNamespace()), run.BOOTSTRAP_CHAIN)
        self.assertEqual(run.source_bootstrap_chain(SimpleNamespace(bootstrap='ert')),
                         ['lisp/romacs/romacs-loadup-bootstrap.el', 'lisp/gnu/help-fns.el',
                          'lisp/romacs/romacs-cl-print.el'])
        self.assertEqual(run.source_bootstrap_chain(SimpleNamespace(bootstrap='ert', harness='mini')),
                         ['lisp/romacs/romacs-loadup-bootstrap.el'])
        for harness in ('mini', 'real'):
            with self.assertRaisesRegex(run.Refusal, 'requires --engine romacs'):
                run.cmd_run(SimpleNamespace(bootstrap='ert', harness=harness, engine='gnu',
                                            harness_fidelity=False))

    def test_selector_does_not_change_mini_regime(self):
        from types import SimpleNamespace
        with self.assertRaisesRegex(run.Refusal, '--selector requires --harness real'):
            run.cmd_run(SimpleNamespace(selector='nil', harness='mini', engine='romacs',
                                        harness_fidelity=False))

    def test_batch_adapter_is_an_explicit_romacs_real_regime(self):
        from types import SimpleNamespace
        for harness, engine, fidelity in [('mini', 'romacs', False),
                                          ('real', 'gnu', False), ('real', 'romacs', True)]:
            with self.assertRaisesRegex(run.Refusal, '--ert-adapter requires --harness real'):
                run.cmd_run(SimpleNamespace(ert_adapter='romacs-batch', harness=harness,
                                            engine=engine, harness_fidelity=fidelity))

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

    def test_fidelity_compares_mini_expected_result_types(self):
        real = {'a': {'outcome': 'failed', 'as_expected': True}}
        mini = {'a': {'outcome': 'errored', 'expected': ':failed', 'as_expected': None}}
        self.assertEqual(self.compare(real, mini), [])
        for wanted in (':passed', 'unsupported'):
            self.assertEqual(len(self.compare(real, {'a': dict(mini['a'], expected=wanted)})), 1)

    def test_scoreboard_process_failure_is_red(self):
        import argparse
        import json
        import tempfile
        import ert_report
        sys.path.insert(0, str(run.ROOT / 'tools/lib'))
        import gnu_oracle
        with tempfile.TemporaryDirectory() as tmp:
            args = argparse.Namespace(all=False, files=['lisp/emacs-lisp/ring-tests.el'],
                                      jobs=1, out=str(pathlib.Path(tmp) / 'report'),
                                      harness_fidelity=False, engine='gnu', bin=None, setup_timeout=10)
            with (patch.object(run, 'verify_pin', return_value={'gnu_commit': run.PIN_COMMIT}),
                  patch.object(run, 'verify_real_lisp'),
                  patch.object(run, 'normalize', side_effect=lambda name: name),
                  patch.object(gnu_oracle, 'require', return_value={'path': '/pinned/emacs'}),
                  patch.object(run, 'load_ledger', return_value={}),
                  patch.object(run, 'ScratchTree', return_value=contextlib.nullcontext()),
                  patch.object(ert_report, 'run_native', side_effect=ert_report.ReportError('truncated ERT summary')),
                  contextlib.redirect_stdout(io.StringIO())):
                status = run.cmd_scoreboard(args)
            self.assertEqual(status, 4, 'REAL-PROCESS: incomplete process must make scoreboard red')
            summary = json.loads((pathlib.Path(args.out) / 'summary.json').read_text())
            self.assertEqual(summary['completed_files'], 0, 'REAL-PROCESS: partial run cannot be complete')
            self.assertEqual(summary['infrastructure_errors'],
                             {'lisp/emacs-lisp/ring-tests.el': 'truncated ERT summary'})

    def test_absence_catches_must_match_fresh_native_data_and_multiplicity(self):
        native = {'outcome': 'passed', 'as_expected': True,
                  'vacuous_void_function': ['(setf immutable-accessor)']}
        rows = run.scoreboard_rows('f', {'a': native}, {'a': native}, 'romacs', {})
        self.assertEqual(rows[0]['verdict'], 'passed')
        self.assertEqual(rows[0]['vacuous_void_function'], ['(setf immutable-accessor)'])
        self.assertEqual(self.compare({'a': native}, {'a': native}), [])
        for catches in [[], ['missing-subject'], ['(setf immutable-accessor)'] * 2]:
            candidate = dict(native, vacuous_void_function=catches)
            rows = run.scoreboard_rows('f', {'a': native}, {'a': candidate}, 'romacs', {})
            self.assertEqual(rows[0]['verdict'], 'failed')
            self.assertEqual(len(self.compare({'a': native}, {'a': candidate})), 1)
        candidate = dict(native, vacuous_void_function=['missing-implementation'])
        reference = dict(native, vacuous_void_function=[])
        rows = run.scoreboard_rows('f', {'a': reference}, {'a': candidate}, 'romacs', {})
        self.assertEqual(rows[0]['verdict'], 'failed')
        self.assertEqual(rows[0]['unmatched_void_function'], ['missing-implementation'])
        self.assertEqual(len(self.compare({'a': native}, {'a': dict(native, as_expected=False)})), 1)

    def test_empty_fidelity_is_not_green(self):
        self.assertTrue(hasattr(run, 'harness_defects'), 'FIDELITY-EMPTY: empty fidelity cannot be green')
        with self.assertRaisesRegex(run.Refusal, 'zero tests', msg='FIDELITY-EMPTY: empty fidelity cannot be green'):
            run.harness_defects({}, {})


if __name__ == '__main__':
    unittest.main()
