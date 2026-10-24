#!/usr/bin/env python3
"""Unit checks for gate resource normalization; native proof uses a CPU timer."""
import importlib.util
import json
import os
import pathlib
import resource
import signal
import subprocess
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('gate_limits', pathlib.Path(__file__).with_name('gate_limits.py'))
limits = importlib.util.module_from_spec(spec)
spec.loader.exec_module(limits)


class ResourceHandoff(unittest.TestCase):
    def setUp(self):
        self.directory = self.enterContext(tempfile.TemporaryDirectory())
        self.runtime = pathlib.Path(self.directory)
        self.runtime.chmod(0o700)
        self.enterContext(mock.patch.dict(os.environ, XDG_RUNTIME_DIR=self.directory,
                                         GATE_TEST_SECRET='not-for-command-lines'))

    def poisoned(self):
        return mock.patch.object(limits.resource, 'getrlimit', return_value=(0, 0))

    def test_nonzero_hard_limits_do_not_launch_or_change_anything(self):
        for value in (resource.RLIM_INFINITY, 1, 200000):
            with self.subTest(value=value), mock.patch.object(limits.resource, 'getrlimit', return_value=(value, value)), \
                    mock.patch.object(limits.subprocess, 'Popen') as popen:
                self.assertIsNone(limits.run(['unused']))
                popen.assert_not_called()

    def test_launch_preserves_context_privately_and_returns_failure_status(self):
        observed = {}
        def launch(command, **kwargs):
            path = pathlib.Path(command[-1])
            observed.update(path=path, state=json.loads(path.read_text()), command=command)
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertTrue(path.is_relative_to(self.runtime))
            self.assertTrue(kwargs['start_new_session'])
            return mock.Mock(wait=mock.Mock(return_value=73), poll=mock.Mock(return_value=73))
        with self.poisoned(), mock.patch.object(limits.subprocess, 'Popen', side_effect=launch):
            self.assertEqual(limits.run(['probe', 'literal $ARG']), 73)
        self.assertEqual(observed['state']['environment'], dict(os.environ))
        self.assertEqual(observed['state']['command'], ['probe', 'literal $ARG'])
        self.assertEqual(observed['state']['cwd'], os.getcwd())
        self.assertEqual(observed['state']['affinity'], sorted(os.sched_getaffinity(0)))
        self.assertIn('-I', observed['command'])
        self.assertIn('--property=LimitRTTIME=infinity', observed['command'])
        self.assertNotIn('not-for-command-lines', ' '.join(observed['command']))
        self.assertFalse(observed['path'].exists())
        self.assertEqual(list(self.runtime.iterdir()), [])

    def test_cancellation_at_launch_stops_exact_unit_and_restores_handlers(self):
        old = signal.getsignal(signal.SIGTERM)
        def launch(*args, **kwargs):
            signal.getsignal(signal.SIGTERM)(signal.SIGTERM, None)
            return mock.Mock(wait=mock.Mock(return_value=0), poll=mock.Mock(return_value=0))
        with self.poisoned(), mock.patch.object(limits.subprocess, 'Popen', side_effect=launch), \
                mock.patch.object(limits.subprocess, 'run') as stop:
            self.assertEqual(limits.run(['probe']), 143)
            self.assertEqual(stop.call_count, 2)
            self.assertTrue(stop.call_args.args[0][-1].startswith('romacs-gate-rlimits-'))
        self.assertEqual(signal.getsignal(signal.SIGTERM), old)
        self.assertEqual(list(self.runtime.iterdir()), [])

    def test_missing_launcher_cleans_handoff_and_handlers(self):
        old = signal.getsignal(signal.SIGINT)
        with self.poisoned(), mock.patch.object(limits.subprocess, 'Popen', side_effect=FileNotFoundError):
            with self.assertRaises(FileNotFoundError):
                limits.run(['probe'])
        self.assertEqual(signal.getsignal(signal.SIGINT), old)
        self.assertEqual(list(self.runtime.iterdir()), [])

    def test_stop_failure_still_settles_child_and_restores_handlers(self):
        old = signal.getsignal(signal.SIGTERM)
        child = mock.Mock(wait=mock.Mock(return_value=0), poll=mock.Mock(return_value=None))
        def launch(*args, **kwargs):
            signal.getsignal(signal.SIGTERM)(signal.SIGTERM, None)
            return child
        with self.poisoned(), mock.patch.object(limits.subprocess, 'Popen', side_effect=launch), \
                mock.patch.object(limits.subprocess, 'run', side_effect=subprocess.TimeoutExpired('stop', 150)):
            with self.assertRaises(subprocess.TimeoutExpired):
                limits.run(['probe'])
        child.wait.assert_called_once_with(timeout=150)
        self.assertEqual(signal.getsignal(signal.SIGTERM), old)

    def test_unsafe_runtime_directory_refuses_before_launch(self):
        self.runtime.chmod(0o755)
        with self.poisoned(), mock.patch.object(limits.subprocess, 'Popen') as popen:
            with self.assertRaisesRegex(ValueError, 'private XDG_RUNTIME_DIR'):
                limits.run(['probe'])
            popen.assert_not_called()

    def test_restore_requires_manager_to_remove_zero_limit(self):
        path = self.runtime / 'handoff'
        path.write_text('{}')
        path.chmod(0o600)
        with self.poisoned(), mock.patch.object(limits.os, 'execvpe') as execute:
            with self.assertRaisesRegex(ValueError, 'did not clear'):
                limits.restore(path)
            execute.assert_not_called()
        self.assertFalse(path.exists())

    def test_restore_rejects_symlink_and_public_handoff(self):
        path = self.runtime / 'handoff'
        path.write_text('{}')
        path.chmod(0o644)
        with self.assertRaisesRegex(ValueError, 'unsafe resource handoff'):
            limits.restore(path)
        link = self.runtime / 'link'
        link.symlink_to(path)
        with self.assertRaises(OSError):
            limits.restore(link)


if __name__ == '__main__':
    unittest.main()
