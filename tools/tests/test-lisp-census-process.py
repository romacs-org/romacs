#!/usr/bin/env python3
"""Focused real-process controls; no VM build, Lisp edits, or census required."""
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import importlib.machinery
import importlib.util
import json
import os
from pathlib import Path
import signal
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('census', _source := ROOT / 'tools/lisp-census', loader=importlib.machinery.SourceFileLoader('census', str(_source)))
assert spec is not None and spec.loader is not None
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)


class ProcessControls(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='census-process-test-', dir=os.environ.get('TMPDIR', str(_PortablePath(_portable_tempfile.gettempdir()) / '')))
        self.addCleanup(self.tmp.cleanup)
        self.directory = Path(self.tmp.name)
        self.path = self.directory / 'target.el'

    def run_control(self, action, report=None):
        # A control that ends gets as long as a loaded host needs to start
        # Python; one that sleeps (10 s) is cut off well before it wakes.
        timeout = 3 if 'sleep' in action else 30
        control = self.directory / 'control.py'
        source = '#!/usr/bin/env python3\nimport os, signal, resource, time\nresource.setrlimit(resource.RLIMIT_CORE, (0, 0))\n'
        if report is not None:
            source += f'print({json.dumps(report)!r}, flush=True)\n'
        control.write_text(source + action + '\n')
        control.chmod(0o700)
        return census.run_file(self.path, {'form_count': 2, 'status': 'ok'}, control, [], timeout)

    def primary(self, ok=False, panic=False):
        return {'measurement': True, 'file': str(self.path), 'ok': ok, 'panic': panic,
                'completed_files': int(ok), 'condition_result': 'void variable: missing',
                'steps': [{'file': str(self.path), 'status': 'ok', 'ordinal': 1},
                          {'file': str(self.path), 'status': 'ok' if ok else 'error',
                           'ordinal': 2, 'error': 'void variable: missing'}]}

    def test_jsonl_unicode_separators_preserve_primary_and_diagnostics(self):
        separators = 'NEL\u0085LINE\u2028PARAGRAPH\u2029'
        primary = self.primary()
        primary['condition_result'] = separators
        primary['steps'][0]['source'] = separators
        candidate = 'CENSUS-MISSING-FUNCTION:missing-candidate ' + separators
        diagnostic = dict(primary, ok=True, completed_files=99, steps=[],
                          diagnostic_condition=separators,
                          candidate_replay=candidate)
        stdout = '\n'.join(json.dumps(report, ensure_ascii=False)
                           for report in [primary, diagnostic]) + '\n'
        expected = dict(primary, diagnostic_condition=separators,
                        candidate_replay=candidate)
        for output in [stdout, stdout.encode('utf-8')]:
            with self.subTest(output_type=type(output).__name__):
                self.assertEqual(census.measurement_output(output), expected)
        # Exercise the actual process boundary too: a later diagnostic cannot
        # turn the authoritative failed primary report into a successful load.
        row = self.run_control(f'print({stdout!r}, end="", flush=True)')
        self.assertTrue(row['primary_report_present'])
        self.assertFalse(row['loaded'])
        self.assertEqual(row['forms_evaluated'], 1)
        self.assertEqual(row['condition_result'], separators)
        self.assertEqual(row['first_failure']['diagnostic_condition'], separators)
        self.assertEqual(row['first_failure']['candidate_missing_function'], 'missing-candidate')
        self.assertEqual(row['exit_code'], 0)

    def test_native_deaths_are_not_panic_evidence(self):
        for sig in [signal.SIGKILL, signal.SIGTERM, signal.SIGABRT]:
            with self.subTest(signal=sig.name):
                row = self.run_control(f'os.kill(os.getpid(), {int(sig)})')
                self.assertEqual(row['first_failure']['classification'], 'process_signal')
                self.assertEqual(row['exit_code'], -sig)
                self.assertEqual(row['signal_number'], sig)
                self.assertEqual(row['signal_name'], sig.name)
                self.assertFalse(row['primary_report_present'])
                self.assertIsNone(row['forms_evaluated'])
        row = self.run_control('raise SystemExit(101)')
        self.assertEqual(row['first_failure']['classification'], 'process_exit')
        self.assertEqual(row['exit_code'], 101)
        self.assertIsNone(row['forms_evaluated'])

    def test_diagnostic_death_preserves_primary_verdict_and_progress(self):
        for ok in [False, True]:
            for action, disposition, code in [
                ('os.kill(os.getpid(), signal.SIGKILL)', 'process_signal', -9),
                ('raise SystemExit(101)', 'process_exit', 101),
                ('time.sleep(10)', 'timeout', -9),
            ]:
                with self.subTest(ok=ok, disposition=disposition):
                    row = self.run_control(action, self.primary(ok=ok))
                    self.assertEqual(row['loaded'], ok)
                    self.assertEqual(row['forms_evaluated'], 2 if ok else 1)
                    self.assertEqual(row['exit_code'], code)
                    self.assertTrue(row['primary_report_present'])
                    self.assertEqual(row['diagnostic_process_disposition'], disposition)
                    if not ok:
                        self.assertEqual(row['first_failure']['classification'], 'missing_variable')
                    else:
                        self.assertNotIn('first_failure', row)

    def test_explicit_primary_panic_is_panic_evidence(self):
        row = self.run_control('raise SystemExit(101)', self.primary(panic=True))
        self.assertEqual(row['first_failure']['classification'], 'vm_panic_abort')
        self.assertEqual(row['process_disposition'], 'process_exit')

    def test_timeout_and_missing_report_have_unknown_progress(self):
        for action, classification in [('time.sleep(10)', 'timeout'), ('pass', 'unknown')]:
            row = self.run_control(action)
            self.assertEqual(row['first_failure']['classification'], classification)
            self.assertIsNone(row['forms_evaluated'])
            self.assertFalse(row['primary_report_present'])
            self.assertEqual(row['supervisor_timeout'], classification == 'timeout')
            self.assertEqual(row['exit_code'], -9 if classification == 'timeout' else 0)

    def test_main_saves_incomplete_evidence_then_returns_nonzero(self):
        row = self.run_control('os.kill(os.getpid(), signal.SIGKILL)')
        output = self.directory / 'incomplete.json'
        binary_dir = self.directory / 'build' / 'release'
        binary_dir.mkdir(parents=True)
        (binary_dir / 'romacs-vm-census').write_bytes(b'not executed: collector fixture')
        # Only external build/oracle discovery is replaced. run_file above uses a
        # real dying child; main performs its real aggregation and atomic write.
        oracle = mock.Mock(stdout=json.dumps({'files': [{'form_count': 2, 'status': 'ok'}]}))
        def discover(command, **kwargs):
            if command[:2] == ['git', 'ls-files']:
                return b''
            return 'fixture-version\n'
        with mock.patch('sys.argv', ['lisp-census.py', '--mode', 'warm', '--files', str(self.path), '--output', str(output)]), \
             mock.patch.dict(os.environ, {'CENSUS_TARGET_DIR': str(binary_dir.parent)}), \
             mock.patch.object(census, 'fingerprint', return_value='fixture-fingerprint'), \
             mock.patch.object(census, 'oracle_inventory', return_value={'gnu_commit': census.GNU_PIN, 'gnu_files': [], 'bundled_paths': []}), \
             mock.patch.object(census, 'gnu_oracle_binary', return_value=str(self.directory / 'fake-gnu-oracle')), \
             mock.patch.object(census.subprocess, 'run', return_value=oracle), \
             mock.patch.object(census.subprocess, 'check_output', side_effect=discover), \
             mock.patch.object(census, 'run_file', return_value=row), \
             mock.patch('builtins.print'):
            self.assertEqual(census.main(), 2)
        evidence = json.loads(output.read_text())
        self.assertFalse(evidence['summary']['measurement_complete'])
        self.assertIsNone(evidence['summary']['forms_evaluated_target_files'])
        self.assertEqual(evidence['summary']['forms_evaluated_target_files_lower_bound'], 0)
        self.assertEqual(evidence['files'][0]['exit_code'], -9)
        self.assertIsNone(evidence['bootstrap_chain'])

    def test_summary_exposes_lower_bound_and_incomplete_measurement(self):
        measured = self.run_control('pass', self.primary())
        unavailable = self.run_control('raise SystemExit(101)')
        for rows, complete, exact in [([measured], True, 1), ([measured, unavailable], False, None), ([unavailable], False, None)]:
            with self.subTest(complete=complete, rows=len(rows)):
                summary = census.summarize(rows, {r['file'] for r in rows}, 'cold')
                self.assertEqual(summary['measurement_complete'], complete)
                self.assertEqual(summary['files_measured'], int(measured in rows))
                self.assertEqual(summary['forms_evaluated_target_files'], exact)
                self.assertEqual(summary['tracked_forms_evaluated'], exact)
                self.assertEqual(summary['forms_evaluated_target_files_lower_bound'], int(measured in rows))
                self.assertEqual(summary['tracked_forms_evaluated_lower_bound'], int(measured in rows))
        self.assertIsNone(census.chain_consensus([unavailable]))


if __name__ == '__main__':
    unittest.main(verbosity=2)
