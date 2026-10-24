#!/usr/bin/env python3
"""Contract checks for scripts/premerge-text; no compiler or slot required."""
import importlib.machinery
import importlib.util
import pathlib
import subprocess
import unittest
from unittest import mock

loader = importlib.machinery.SourceFileLoader(
    'premerge_text', str(pathlib.Path(__file__).with_name('premerge-text')))
premerge = importlib.util.module_from_spec(importlib.util.spec_from_loader('premerge_text', loader))
loader.exec_module(premerge)


class PremergeText(unittest.TestCase):
    def test_release_scope_and_cfg_verified_before_tests(self):
        with mock.patch.object(premerge.subprocess, 'run', return_value=mock.Mock(
                stdout='target_arch="x86_64"\ntest\n')) as run:
            premerge.main()
        self.assertEqual([c.args[0] for c in run.call_args_list], [
            ['cargo', 'rustc', '--offline', '--release', '-p', 'romacs-text',
             '--lib', '--', '--print', 'cfg'],
            ['cargo', 'rustc', '--offline', '--release', '-p', 'romacs-text',
             '--test', 'structural', '--', '--print', 'cfg'],
            ['cargo', 'nextest', 'run', '--offline', '--release', '-p', 'romacs-text',
             '-p', 'romacs-chars', '--no-tests=fail']])
        for call in run.call_args_list:
            self.assertTrue(call.kwargs['check'])
            self.assertEqual(call.kwargs['cwd'], premerge.ROOT)

    def test_debug_assertions_or_missing_cfg_refuses_before_tests(self):
        for bad in ('debug_assertions\ntarget_arch="x86_64"\n', ''):
            for target in (0, 1):
                results = [mock.Mock(stdout='target_arch="x86_64"\n'), mock.Mock(stdout=bad)]
                with self.subTest(cfg=bad, target=target), mock.patch.object(
                        premerge.subprocess, 'run', side_effect=results[1 - target:]) as run:
                    with self.assertRaisesRegex(SystemExit, 'debug_assertions off'):
                        premerge.main()
                self.assertEqual(run.call_count, target + 1)

    def test_compiler_or_test_failure_propagates(self):
        for failure_at in range(3):
            results = [mock.Mock(stdout='target_arch="x86_64"\n')] * failure_at
            results.append(subprocess.CalledProcessError(1, ['cargo']))
            with self.subTest(failure_at=failure_at), mock.patch.object(
                    premerge.subprocess, 'run', side_effect=results):
                with self.assertRaises(subprocess.CalledProcessError):
                    premerge.main()


if __name__ == '__main__':
    unittest.main()
