#!/usr/bin/env python3
"""Source-only tests: apply the proposed patch in scratch, never run a gate."""
import os as _portable_os
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import contextlib
import importlib.util
import io
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


SOURCE = Path(_portable_os.environ['ROMACS_ATTESTATION_SOURCE'])
PATCH = Path(__file__).with_name('attestation-controls.patch')
SCRATCH = Path(str(_PortablePath(_portable_tempfile.gettempdir()) / ''))
sys.dont_write_bytecode = True


class ControlsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workspace = tempfile.TemporaryDirectory(prefix='attctl-test-', dir=SCRATCH)
        cls.addClassCleanup(cls.workspace.cleanup)
        cls.root = Path(cls.workspace.name)
        shutil.copyfile(SOURCE, cls.root / SOURCE.name)
        shutil.copyfile(SOURCE.with_name('romacs-receipt-check.py'),
                        cls.root / 'romacs-receipt-check.py')
        subprocess.run(['patch', '--batch', '--fuzz=0', '-p1', '-i', str(PATCH)],
                       cwd=cls.root, check=True, capture_output=True)
        spec = importlib.util.spec_from_file_location('attctl_subject', cls.root / SOURCE.name)
        cls.subject = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.subject)

    def argv(self, *extra):
        return ['produce', '--repo', str(self.root / 'repo'), '--master', 'a' * 40,
                '--label', 'T1', '--output', str(self.root / 'output.json'), *extra]

    def invoke(self, *extra):
        with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
            return self.subject.main(self.argv(*extra))

    def assert_forwarded(self, extra, jobs, threads):
        subject = self.subject
        runs = self.root / 'runs'
        runs.mkdir(exist_ok=True)
        new_run = runs / 'new'
        target = self.root / 'target'
        target.mkdir(exist_ok=True)
        bound = {'gnu_pin': 'b' * 40}
        commands = []

        def lane(repo, label, command, *, oracle=None):
            self.assertEqual(label, 'T1')
            commands.append(command)
            if 'full' in command:
                return f'gate: GREEN: full (1s); logs in {new_run}\n'.encode()
            return b''

        with contextlib.ExitStack() as stack:
            stack.enter_context(mock.patch.dict(subject.os.environ,
                                                 {'LANE_TARGET_OVERRIDE': str(target)}))
            stack.enter_context(mock.patch.object(subject, 'RUNS', runs))
            stack.enter_context(mock.patch.object(subject, 'SCRATCH', self.root))
            stack.enter_context(mock.patch.object(subject, 'git', return_value=b'c' * 40))
            stack.enter_context(mock.patch.object(subject, 'clean'))
            stack.enter_context(mock.patch.object(subject, 'oid', return_value='a' * 40))
            stack.enter_context(mock.patch.object(subject, 'bindings', return_value=bound))
            stack.enter_context(mock.patch.object(subject, 'lane', side_effect=lane))
            check = stack.enter_context(mock.patch.object(
                subject, 'receipt_check', side_effect=subject.Refused('validator reached')))
            sign = stack.enter_context(mock.patch.object(subject, 'run'))
            self.assertEqual(self.invoke(*extra), 1)
            check.assert_called_once_with(new_run / 'receipt.json', 'T1', bound)
            sign.assert_not_called()
        full = [command for command in commands if 'full' in command]
        self.assertEqual(full, [[
            'bash', str(subject.HERE / 'gate-slot.sh'), 'run', 'T1', '--', 'python3',
            'scripts/gate', '--lane', 'T1', 'full', '--jobs', str(jobs),
            '--display-threads', str(threads), '--mem', '34G']])
        self.assertFalse((self.root / 'output.json').exists())

    def test_explicit_controls_reach_full_gate_and_existing_validator(self):
        self.assert_forwarded(['--jobs', '4', '--display-threads', '8'], 4, 8)

    def test_defaults_preserve_jobs5_display4(self):
        self.assert_forwarded([], 5, 4)

    def test_invalid_controls_reject_before_any_producer_effect(self):
        subject = self.subject
        for option in ('--jobs', '--display-threads'):
            for value in ('0', '-1', '1.5', 'invalid'):
                with self.subTest(option=option, value=value), contextlib.ExitStack() as stack:
                    boundaries = [stack.enter_context(mock.patch.object(subject, name))
                                  for name in ('git', 'clean', 'oid', 'bindings', 'lane',
                                               'receipt_check', 'run', 'read', 'verify')]
                    allocate = stack.enter_context(mock.patch.object(subject.tempfile, 'mkdtemp'))
                    resolve = stack.enter_context(mock.patch.object(subject.Path, 'resolve'))
                    write = stack.enter_context(mock.patch.object(subject.Path, 'open'))
                    with self.assertRaises(SystemExit) as failure:
                        self.invoke(option, value)
                    self.assertEqual(failure.exception.code, 2)
                    for boundary in [*boundaries, allocate, resolve, write]:
                        boundary.assert_not_called()

    def test_supplied_receipt_still_rejected_before_effects(self):
        subject = self.subject
        with contextlib.ExitStack() as stack:
            boundaries = [stack.enter_context(mock.patch.object(subject, name))
                          for name in ('git', 'lane', 'receipt_check', 'run')]
            allocate = stack.enter_context(mock.patch.object(subject.tempfile, 'mkdtemp'))
            self.assertEqual(self.invoke('--receipt', '/unsigned/receipt.json',
                                         '--jobs', '4', '--display-threads', '8'), 1)
            for boundary in [*boundaries, allocate]:
                boundary.assert_not_called()


if __name__ == '__main__':
    unittest.main()
