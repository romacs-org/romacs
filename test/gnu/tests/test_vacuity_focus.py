"""Focused observation parsing must not turn missing execution into evidence."""
import importlib.util
from pathlib import Path
import unittest

PATH = Path(__file__).resolve().parents[1] / 'tools' / 'probe-vacuity-focus.py'


class FocusTests(unittest.TestCase):
    def test_missing_completion_is_rejected(self):
        self.assertTrue(PATH.is_file(), 'focused probe producer must exist')
        spec = importlib.util.spec_from_file_location('focus', PATH)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with self.assertRaisesRegex(ValueError, 'completion'):
            module.parse_vm('{"event":"result","test":"x","outcome":"passed"}\n', 'x')

    def test_gnu_error_is_not_an_assertion_kill(self):
        spec = importlib.util.spec_from_file_location('focus', PATH)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with self.assertRaisesRegex(ValueError, 'assertion'):
            module.parse_gnu('VCHAIN|x|errored|1|0|1\n', 'x')

    def test_gnu_inconsistent_counts_refused(self):
        spec = importlib.util.spec_from_file_location('focus', PATH)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with self.assertRaisesRegex(ValueError, 'counts'):
            module.parse_gnu('VCHAIN|x|failed|1|1|0\n', 'x')

    def test_wrong_identity_is_rejected(self):
        self.assertTrue(PATH.is_file(), 'focused probe producer must exist')
        spec = importlib.util.spec_from_file_location('focus', PATH)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with self.assertRaisesRegex(ValueError, 'identity'):
            module.parse_vm('{"event":"result","test":"other","outcome":"passed"}\n'
                            '{"event":"done","registered":1}\n', 'x')


if __name__ == '__main__':
    unittest.main()
