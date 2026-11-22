#!/usr/bin/env python3
"""Discovery, ownership and verdicts of tools/lib/python_tests.py."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import python_tests  # noqa: E402


class Runner(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)

    def write(self, path, text):
        file = self.root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(text)
        subprocess.run(['git', '-C', str(self.root), 'add', path], check=True)

    def test_discovery_takes_tracked_unittest_files_outside_the_lisp_trees(self):
        case = 'import unittest\nclass T(unittest.TestCase):\n    def test_x(self): pass\n'
        for path in ('tools/test_a.py', 'scripts/b_test.py', 'scripts/test-c.py',
                     'test/gnu/test/lisp/test_gnu.py', 'lisp/test_d.py', 'tools/test_tool.py'):
            self.write(path, case if path != 'tools/test_tool.py' else 'print("a tool")\n')
        self.assertEqual(python_tests.test_files(self.root),
                         ['scripts/b_test.py', 'scripts/test-c.py', 'tools/test_a.py'])

    def test_an_owner_that_does_not_name_its_file_is_refused(self):
        self.write('runner.rs', 'python3 -m unittest test_owned')
        with mock.patch.dict(python_tests.ELSEWHERE, {'tools/test_owned.py': 'runner.rs'}, clear=True):
            self.assertEqual(python_tests.unowned(self.root, ['tools/test_owned.py', 'tools/test_b.py']),
                             ['tools/test_b.py'])
            with self.assertRaisesRegex(ValueError, 'does not run it'):
                python_tests.unowned(self.root, ['tools/test_b.py'])
        with mock.patch.dict(python_tests.ELSEWHERE, {'tools/test_b.py': 'runner.rs'}, clear=True):
            with self.assertRaisesRegex(ValueError, 'does not run it'):
                python_tests.unowned(self.root, ['tools/test_b.py'])

    def test_a_file_passes_only_when_test_bodies_ran_and_passed(self):
        self.write('tools/test_pass.py', 'import unittest\nclass T(unittest.TestCase):\n    def test_x(self): pass\n')
        self.write('tools/test_fail.py', 'import unittest\nclass T(unittest.TestCase):\n    def test_x(self): self.fail()\n')
        self.write('tools/test_empty.py', 'import unittest\n')
        self.write('tools/test_skipped.py', 'import unittest\n@unittest.skip("no oracle")\nclass T(unittest.TestCase):\n'
                   '    def test_x(self): pass\n    def test_y(self): pass\n')
        verdicts = {path: ok for path, ok, _, _ in
                    (python_tests.run(self.root, f'tools/test_{name}.py') for name in ('pass', 'fail', 'empty', 'skipped'))}
        self.assertEqual(verdicts, {'tools/test_pass.py': True, 'tools/test_fail.py': False,
                                    'tools/test_empty.py': False, 'tools/test_skipped.py': False})

    def test_a_file_that_cannot_be_started_is_not_run_rather_than_failed(self):
        self.write('tools/test_pass.py', 'import unittest\nclass T(unittest.TestCase):\n    def test_x(self): pass\n')
        with mock.patch.object(python_tests, 'command', return_value=[str(self.root / 'no-such-python')]):
            path, ok, _, tail = python_tests.run(self.root, 'tools/test_pass.py')
        self.assertIsNone(ok)
        self.assertIn('could not run it', tail)

    def test_a_failure_to_clean_up_after_a_failed_file_keeps_the_failure(self):
        self.write('tools/test_fail.py', 'import unittest\nclass T(unittest.TestCase):\n    def test_x(self): self.fail()\n')
        real = subprocess.run
        def run(argv, *args, **kwargs):
            if argv[0] == 'chmod':
                raise OSError('no chmod')
            return real(argv, *args, **kwargs)
        with mock.patch.object(python_tests.subprocess, 'run', side_effect=run):
            _, ok, _, _ = python_tests.run(self.root, 'tools/test_fail.py')
        self.assertIs(ok, False)

    def test_output_that_is_not_utf_8_is_still_a_verdict(self):
        self.write('tools/test_bytes.py', 'import os, sys, unittest\nclass T(unittest.TestCase):\n'
                   '    def test_x(self):\n        os.write(2, b"\\xff\\xfe")\n        self.fail()\n')
        _, ok, _, tail = python_tests.run(self.root, 'tools/test_bytes.py')
        self.assertIs(ok, False)
        self.assertIn('\ufffd', tail)

    def test_a_tracked_name_that_is_not_utf_8_does_not_stop_discovery(self):
        odd = self.root / os.fsdecode(b'notes-\xff.txt')
        odd.write_text('x')
        subprocess.run(['git', '-C', str(self.root), 'add', '--', odd.name], check=True)
        self.write('tools/test_a.py', 'import unittest\n')
        self.assertEqual(python_tests.test_files(self.root), ['tools/test_a.py'])

    def test_files_the_host_gives_no_worker_for_are_not_run_and_the_rest_keep_their_verdicts(self):
        import threading
        real, started = threading.Thread.start, []
        gate = threading.Event()
        def start(thread):
            started.append(thread)
            if len(started) > 1:
                gate.set()  # the busy worker goes on to what is queued
                raise RuntimeError("can't start new thread")
            return real(thread)
        def run(root, path):
            gate.wait(5)  # the one worker is busy while the second is refused
            return path, False, 0.0, ''
        with mock.patch.object(python_tests, 'run', run), mock.patch.object(threading.Thread, 'start', start):
            results = python_tests.run_all(self.root, ['a', 'b', 'c'], 2)
        self.assertEqual([(path, ok) for path, ok, _, _ in results], [('a', False), ('b', False), ('c', None)])
        self.assertEqual(python_tests.verdict(results), 1)

    def test_the_exit_is_1_for_a_failure_2_for_the_runner_s_own_and_0_otherwise(self):
        def results(*oks):
            return [(f'f{n}', ok, 0.0, '') for n, ok in enumerate(oks)]
        self.assertEqual(python_tests.verdict(results(True, True)), 0)
        self.assertEqual(python_tests.verdict(results(True, None)), 2)
        self.assertEqual(python_tests.verdict(results(None, False, True)), 1)
        self.assertEqual(python_tests.verdict([]), 0)

    def test_a_root_git_cannot_list_exits_2(self):
        with mock.patch.object(python_tests.sys, 'argv', ['python_tests.py', '--root', str(self.root / 'absent')]):
            self.assertEqual(python_tests.main(), 2)


if __name__ == '__main__':
    unittest.main()
