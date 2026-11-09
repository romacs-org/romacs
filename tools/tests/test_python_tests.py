#!/usr/bin/env python3
"""Discovery, ownership and verdicts of tools/lib/python_tests.py."""
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
                     'test/lisp/test_gnu.py', 'gnu-lisp/test_d.py', 'tools/test_tool.py'):
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

    def test_a_file_passes_only_when_tests_ran_and_passed(self):
        self.write('tools/test_pass.py', 'import unittest\nclass T(unittest.TestCase):\n    def test_x(self): pass\n')
        self.write('tools/test_fail.py', 'import unittest\nclass T(unittest.TestCase):\n    def test_x(self): self.fail()\n')
        self.write('tools/test_empty.py', 'import unittest\n')
        verdicts = {path: ok for path, ok, _, _ in
                    (python_tests.run(self.root, f'tools/test_{name}.py') for name in ('pass', 'fail', 'empty'))}
        self.assertEqual(verdicts, {'tools/test_pass.py': True, 'tools/test_fail.py': False,
                                    'tools/test_empty.py': False})


if __name__ == '__main__':
    unittest.main()
