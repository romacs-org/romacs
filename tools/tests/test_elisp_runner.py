"""Public-entry controls for the source-loaded Elisp test runner."""
import hashlib
import importlib.machinery
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

    def test_an_ert_file_is_refused_before_any_host_runs(self):
        ran = Path(self.temp.name) / 'ran'
        fake = Path(self.temp.name) / 'fake-host'
        fake.write_text(f'#!/bin/sh\ntouch {ran}\nexit 0\n')
        fake.chmod(0o700)
        for text in ['(ert-deftest an-ert-test () (should t))\n', '(defun no-tests-here () nil)\n',
                     # The only test commented out leaves the file none.
                     ';; (romacs-deftest fixture-pass () (romacs-should t))\n(defun no-tests-here () nil)\n']:
            self.fixture.write_text(text)
            result = self.run_tests(binary=fake)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('tools/test-ert', result.stderr)
            self.assertFalse(ran.exists(), 'the host ran for a file the runner should refuse')

    def test_forms_inside_strings_comments_and_characters_are_not_read(self):
        self.fixture.write_text('''(romacs-deftest fixture-pass ()
  (romacs-should (equal (list ?\\" ?\\( ?;) (list 34 40 59)))
  (romacs-should (stringp "a string that shows
(ert-deftest not-a-test () t)
in its text")))
''')
        result = self.run_tests('--test', 'fixture-pass')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('PASS fixture-pass', result.stdout)

    def test_the_editor_header_selects_files_only_from_the_leading_comments(self):
        runner = importlib.machinery.SourceFileLoader('test_elisp', str(ROOT / 'tools/test-elisp')).load_module()
        plain = '(romacs-deftest a () (romacs-should t))\n'
        marked = ';;; f.el --- x\n;; romacs-test: editor (needs the editor)\n' + plain
        late = plain + ';; romacs-test: editor\n'
        files, texts = ['plain', 'marked', 'late'], [plain, marked, late]
        # The bare VM's default set skips the marked file; editor mode runs it.
        self.assertEqual(runner.selection(files, texts, core=True, named=False), (['plain', 'late'], None))
        self.assertEqual(runner.selection(files, texts, core=False, named=False), (files, None))
        # Naming a marked file to the bare VM is an error, not a skip.
        _, problem = runner.selection(['marked'], [marked], core=True, named=True)
        self.assertIn('romacs-test: editor', problem)

    def test_core_refuses_a_file_that_declares_it_needs_the_editor(self):
        self.fixture.write_text(';; romacs-test: editor\n(romacs-deftest fixture-pass () (romacs-should t))\n')
        result = self.run_tests('--test', 'fixture-pass')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('romacs-test: editor', result.stderr)


if __name__ == '__main__':
    unittest.main()
