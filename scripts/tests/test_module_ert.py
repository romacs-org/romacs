"""Exercise the module ERT command with real GNU ERT and negative hosts."""
import hashlib
import os
import runpy
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ModuleERT(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        binary = os.environ.get('ROMACS_GNU_ORACLE')
        if not binary or not Path(binary).is_file():
            raise RuntimeError('Set ROMACS_GNU_ORACLE to the pinned GNU executable')
        cls.binary = Path(binary).resolve()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.fixture = self.home / 'tests.el'
        self.fixture.write_text(''';;; -*- lexical-binding: t -*-
(require 'ert)
(ert-deftest fixture-pass () (should (= 2 (+ 1 1))))
(ert-deftest fixture-fail () (should (= 2 3)))
(ert-deftest fixture-error () (error "ordinary fixture error"))
(ert-deftest fixture-skip () (ert-skip "fixture"))
''')

    def invoke(self, *args, binary=None, make=False):
        env = os.environ.copy()
        env.update(MODULE='', SELECTOR='t', TEST_FILES='', TEST_TIMEOUT='10',
                   ROMACS_TEST_BINARY=str(binary or self.binary))
        if make:
            command = ['make', '--no-print-directory', 'test-elisp',
                       'MODULE=gnosis-agent-romacs', *args]
        else:
            command = [sys.executable, str(ROOT / 'scripts/test-ert'),
                       '--file', str(self.fixture), *args]
        return subprocess.run(command, cwd=ROOT, env=env, capture_output=True,
                              text=True, timeout=20)

    def test_real_ert_pass_failure_error_and_skips(self):
        for name, success in [('pass', True), ('fail', False), ('error', False)]:
            with self.subTest(name=name):
                result = self.invoke('--selector', 'fixture-' + name)
                self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
                self.assertIn('selected=1 started=1 completed=1', result.stdout)
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('passed=1 failed=2 skipped=1', result.stdout)
        result = self.invoke('--selector', 'fixture-skip')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('All selected tests skipped', result.stderr)

    def test_empty_invalid_selectors_and_load_errors(self):
        for selector in ['"absent"', '(invalid-selector)', '"["']:
            with self.subTest(selector=selector):
                result = self.invoke('--selector', selector)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('ERT incomplete', result.stderr)
        self.fixture.write_text('(error "suite load failure")')
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('suite load failure', result.stdout)

    def test_missing_and_zero_exit_hosts_fail(self):
        result = self.invoke(binary=self.home / 'missing')
        self.assertNotEqual(result.returncode, 0)
        fake = self.home / 'empty-host'
        fake.write_text('#!/bin/sh\nexit 0\n')
        fake.chmod(0o700)
        result = self.invoke(binary=fake)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('incomplete ERT report', result.stderr)

    def test_source_changes_without_host_rebuild(self):
        before = hashlib.sha256(self.binary.read_bytes()).digest()
        self.assertEqual(self.invoke('--selector', 'fixture-pass').returncode, 0)
        self.fixture.write_text('(ert-deftest fixture-pass () (should nil))')
        result = self.invoke('--selector', 'fixture-pass')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('failed=1', result.stdout)
        self.assertEqual(before, hashlib.sha256(self.binary.read_bytes()).digest())

    def test_make_launches_real_module_suite(self):
        result = self.invoke(make=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('gnosis-test-agent-romacs.el', result.stdout)
        self.assertIn('ERT complete:', result.stdout)

    def test_unknown_module_and_budget_refuse(self):
        result = self.invoke(make=True, *['MODULE=unknown-module'])
        self.assertNotEqual(result.returncode, 0)
        for timeout in ['0', '60', 'nan', 'inf']:
            self.assertNotEqual(self.invoke('--timeout', timeout).returncode, 0)

    def test_package_selection_keeps_suites_and_leaves_support_to_require(self):
        select = runpy.run_path(str(ROOT / 'scripts/test-ert'))['test_files']
        names = {path.name for path in select('gnosis')}
        self.assertIn('gnosis-test-agent-romacs.el', names)
        self.assertIn('gnosis-test-runner.el', names)
        self.assertFalse(names & {'gnosis-test-db.el', 'gnosis-test-helpers.el',
                                  'gnosis-test-schema-v8.el'})

    def test_timeout_kills_owned_descendants(self):
        for detached in [False, True]:
            with self.subTest(detached=detached):
                self.assert_timeout_cleanup(detached)

    def assert_timeout_cleanup(self, detached):
        fake = self.home / 'hanging-host'
        pidfile = self.home / 'child.pid'
        fake.write_text(f'''#!{sys.executable}
import pathlib, subprocess, time
child = subprocess.Popen(["sleep", "30"], start_new_session={detached!r})
pathlib.Path({str(pidfile)!r}).write_text(str(child.pid))
time.sleep(30)
''')
        fake.chmod(0o700)
        started = time.monotonic()
        result = self.invoke('--timeout', '1', binary=fake)
        self.assertNotEqual(result.returncode, 0)
        self.assertLess(time.monotonic() - started, 5)
        self.assertTrue(pidfile.is_file())
        pid = int(pidfile.read_text())
        state = Path(f'/proc/{pid}/stat')
        if state.exists():
            self.assertEqual(state.read_text().split()[2], 'Z')
        else:
            with self.assertRaises(ProcessLookupError):
                os.kill(pid, signal.SIGCONT)


if __name__ == '__main__':
    unittest.main()
