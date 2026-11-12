"""Real GNU ERT bodies and failure controls on the embedded runtime."""
import json
import io
import os
from pathlib import Path
import select
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ert_report
import run


class RealRomacs(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.binary = Path(os.environ['ROMACS_GNU_ERT'])
        self.environment = run.gnu_env(self.home)

    def command(self, source, selector='t'):
        target = self.home / 'fixture.el'
        target.write_text(source)
        plan = self.home / 'plan.json'
        plan.write_text(json.dumps(dict(file=str(target), forms=[])))
        command = [str(self.binary), '--harness-kind', 'real', '--bootstrap-regime', 'gnu-ert',
                   '--harness', str(run.ROOT / 'lisp/gnu/emacs-lisp/ert.el'),
                   '--plan', str(plan), '--selector', selector, '--ert-adapter', 'romacs-batch']
        for path in run.source_bootstrap_chain(SimpleNamespace(bootstrap='ert', harness='real')):
            command += ['--preload', str(run.ROOT / path)]
        for directory in run.load_path() + [str(run.ROOT / 'lisp/romacs')]:
            command += ['--load-path', directory]
        return command

    def execute(self, source, selector='t'):
        process = subprocess.run(self.command(source, selector), env=self.environment,
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                 text=True, timeout=90)
        try:
            report = ert_report.parse_report(process.stdout, process.returncode)
        except ert_report.ReportError as error:
            error.add_note(process.stdout)
            raise
        return process, report

    def test_unchanged_representative_files_through_real_runner(self):
        files = {'lisp/emacs-lisp/thunk-tests.el': 9,
                 'lisp/emacs-lisp/ring-tests.el': 24,
                 'lisp/emacs-lisp/let-alist-tests.el': 8}
        out = self.home / 'report'
        command = [sys.executable, str(run.HERE / 'run.py'), 'run', '--harness', 'real',
                   '--bootstrap', 'ert', '--ert-adapter', 'romacs-batch', '--bin', str(self.binary), '--jobs', '1',
                   '--out', str(out), *files]
        process = subprocess.run(command, env=self.environment, capture_output=True,
                                 text=True, timeout=180)
        self.assertEqual(process.returncode, 0, process.stdout + process.stderr)
        summary = json.loads((out / 'summary.json').read_text())
        self.assertEqual(summary['completed_files'], 3)
        self.assertEqual(summary['bootstrap_regime'], 'gnu-ert')
        self.assertEqual(summary['ert_adapter']['name'], 'romacs-batch')
        self.assertFalse(summary['is_unmodified_gnu_ert'])
        self.assertEqual(summary['ert_totals'], dict(passed=41, failed=0, skipped=0, total=41))
        for file, count in files.items():
            evidence = summary['execution_reports'][file]
            self.assertTrue(evidence['complete'])
            self.assertEqual(evidence['execution'], dict(selected=count, started=count, terminal=count))

    def test_reduced_thunk_keeps_its_floor_with_complete_prerequisites(self):
        file = 'lisp/emacs-lisp/thunk-tests.el'
        plan = run.extract(file, self.home)
        result = run.run_romacs(self.binary, file, plan, self.home, 60, 120, io.StringIO(),
                                bootstrap='ert')
        self.assertIsNone(result['fatal'])
        self.assertEqual(len(result['results']), 9)
        self.assertEqual({row['outcome'] for row in result['results'].values()}, {'passed'})
        forms = [event for event in result['events'] if event.get('event') == 'form']
        self.assertEqual(len(forms), len(plan['forms']))
        self.assertTrue(all(event['ok'] for event in forms))
        chain = next(event for event in result['events'] if event.get('event') == 'chain')
        self.assertEqual(chain['regime'], 'gnu-ert')
        self.assertTrue(all(entry['loaded'] for entry in chain['entries']))
        protected = result['results']['thunk-let-bound-vars-cant-be-set-test']
        self.assertEqual(protected['caught'], [['void-function', '(setf thunk-force)']])

    def test_expected_errors_and_cleanup(self):
        process, report = self.execute('''
(defvar real-ert-cleaned nil)
(defvar real-ert-buffer nil)
(ert-deftest a-cleanup ()
  (setq real-ert-buffer (current-buffer))
  (unwind-protect (should-error (car 1) :type 'wrong-type-argument)
    (setq real-ert-cleaned t)))
(ert-deftest b-cleanup-observed ()
  (should real-ert-cleaned)
  (should-not (buffer-live-p real-ert-buffer)))
(ert-deftest c-failure-cleanup ()
  :expected-result :failed
  (setq real-ert-buffer (current-buffer))
  (unwind-protect (should nil) (setq real-ert-cleaned 'after-failure)))
(ert-deftest d-failure-cleanup-observed ()
  (should (eq real-ert-cleaned 'after-failure))
  (should-not (buffer-live-p real-ert-buffer)))
''')
        self.assertEqual(process.returncode, 0, process.stdout)
        self.assertEqual(report['summary'], dict(passed=3, failed=1, skipped=0, total=4))
        self.assertEqual(report['execution'], dict(selected=4, started=4, terminal=4))
        self.assertTrue(report['results']['c-failure-cleanup']['as_expected'])

    def test_selection_and_failed_assertion(self):
        source = '(ert-deftest selected-a () (should nil))\n(ert-deftest selected-b () (should t))'
        process, report = self.execute(source, "'selected-b")
        self.assertEqual(process.returncode, 0, process.stdout)
        self.assertEqual(report['selected'], ['selected-b'])
        self.assertEqual(report['execution'], dict(selected=1, started=1, terminal=1))
        process, report = self.execute(source, "'selected-a")
        self.assertEqual(process.returncode, 1, process.stdout)
        self.assertEqual(report['results']['selected-a'], dict(outcome='failed', as_expected=False))
        self.assertIn('Test selected-a backtrace:', process.stdout)
        self.assertIn('ert-fail', process.stdout)
        self.assertIn('Test selected-a condition:', process.stdout)
        self.assertIn('(ert-test-failed', process.stdout)
        self.assertNotIn('cl-prin1:', process.stdout)
        self.assertNotIn('Error in backtrace printer:', process.stdout)

    def test_zero_selection_is_not_success(self):
        process = subprocess.run(self.command('(ert-deftest present () (should t))', '"no-such-test"'),
                                 env=self.environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                 text=True, timeout=90)
        self.assertIn('Running 0 tests', process.stdout)
        self.assertNotEqual(process.returncode, 0, 'ERT-ZERO-SELECTION-EXIT\n' + process.stdout)
        with self.assertRaises(ert_report.ReportError) as caught:
            ert_report.parse_report(process.stdout, process.returncode)
        self.assertEqual(caught.exception.evidence['execution'], dict(selected=0, started=0, terminal=0))

    def interrupt(self, source, marker):
        process = subprocess.Popen(self.command(source), env=self.environment,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        output = bytearray()
        pending = bytearray()
        deadline = time.monotonic() + 300
        selected = False
        prefix = ert_report.PREFIX.encode()
        observed = False
        try:
            while time.monotonic() < deadline and not observed:
                if not select.select([process.stdout], [], [], max(0, deadline - time.monotonic()))[0]:
                    break
                block = os.read(process.stdout.fileno(), 65536)
                if not block:
                    break
                output.extend(block)
                pending.extend(block)
                while b'\n' in pending:
                    line, _, pending = pending.partition(b'\n')
                    if not selected and line.startswith(prefix):
                        event = json.loads(line[len(prefix):])
                        if event.get('event') == 'selected':
                            selected = True
                            deadline = time.monotonic() + 90
                    observed |= line == marker.encode()
            self.assertTrue(observed, output.decode(errors='replace'))
        finally:
            if process.poll() is None:
                process.kill()
            output.extend(process.communicate(timeout=10)[0])
        with self.assertRaises(ert_report.ReportError) as caught:
            ert_report.parse_report(output.decode(errors='replace'), process.returncode)
        return caught.exception.evidence

    def test_interrupted_body_is_incomplete(self):
        evidence = self.interrupt('''
(ert-deftest interrupted () (message "ERT-BODY-ENTERED") (while t))
''', 'ERT-BODY-ENTERED')
        self.assertEqual(evidence['execution'], dict(selected=1, started=1, terminal=0))
        self.assertFalse(evidence['complete'])

    def test_death_after_all_bodies_cannot_complete(self):
        evidence = self.interrupt('''
(ert-deftest completed () (should t))
(advice-add 'ert-run-tests-batch :after
            (lambda (&rest _) (message "ERT-BATCH-RETURNED") (while t)))
''', 'ERT-BATCH-RETURNED')
        self.assertEqual(evidence['execution'], dict(selected=1, started=1, terminal=1))
        self.assertFalse(evidence['complete'])


if __name__ == '__main__':
    unittest.main()
