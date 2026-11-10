"""Public-entry controls for the source-loaded Elisp test runner."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ElispRunner(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target'))
        cls.binary = Path(os.environ.get('ROMACS_CORE_TEST_BINARY', target / 'gate/romacs-vm')).resolve()
        if not cls.binary.is_file():
            raise RuntimeError('Build romacs-vm once or set ROMACS_CORE_TEST_BINARY')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.temp.cleanup)
        self.fixture = Path(self.temp.name) / 'fixture.el'
        self.fixture.write_text(''';;; -*- lexical-binding: t -*-
(romacs-deftest fixture-pass () (romacs-should (= 2 (+ 1 1))))
(romacs-deftest fixture-fail () (romacs-should (= 2 3)))
(romacs-deftest fixture-error () (error "ordinary fixture error"))
''')

    def run_tests(self, *args, binary=None):
        return subprocess.run([str(ROOT / 'tools/test-elisp'), '--core', '--binary', str(binary or self.binary), str(self.fixture), *args], capture_output=True, text=True, timeout=10)

    def test_pass_fail_and_error_exit_statuses(self):
        for name, status in [('fixture-pass', 'PASS'), ('fixture-fail', 'FAIL'), ('fixture-error', 'ERROR')]:
            result = self.run_tests('--test', name)
            self.assertEqual(result.returncode == 0, status == 'PASS', result.stdout + result.stderr)
            self.assertIn(f'{status} {name}', result.stdout)
        result = self.run_tests()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('SUMMARY total=3 passed=1 failed=1 errors=1', result.stdout)

    def test_empty_unknown_and_invalid_selectors_fail(self):
        for option, selector in [('--match', 'absent'), ('--test', 'absent'), ('--match', '[')]:
            result = self.run_tests(option, selector)
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_zero_exit_without_execution_is_not_green(self):
        fake = Path(self.temp.name) / 'fake-host'
        fake.write_text('#!/bin/sh\nexit 0\n')
        fake.chmod(0o700)
        result = self.run_tests('--test', 'fixture-pass', binary=fake)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('without completing', result.stderr)

    def test_source_edit_is_seen_without_rebuilding_host(self):
        before = hashlib.sha256(self.binary.read_bytes()).digest()
        self.assertEqual(self.run_tests('--test', 'fixture-pass').returncode, 0)
        self.fixture.write_text('(romacs-deftest fixture-pass () (romacs-should nil))\n')
        result = self.run_tests('--test', 'fixture-pass')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('FAIL fixture-pass', result.stdout)
        self.assertEqual(before, hashlib.sha256(self.binary.read_bytes()).digest())


if __name__ == '__main__':
    unittest.main()
