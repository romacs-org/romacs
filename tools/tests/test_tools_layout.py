#!/usr/bin/env python3
"""tools/ is the one place for scripts: commands at its top without .py,
Python modules in tools/lib/, tests in tools/tests/; no scripts/ beside it."""
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]


def tracked(*paths):
    return subprocess.check_output(['git', '-C', str(ROOT), 'ls-files', *paths], text=True).split()


class ToolsLayout(unittest.TestCase):
    def test_no_scripts_directory(self):
        self.assertEqual(tracked('scripts'), [])

    def test_commands_at_the_top_carry_no_py_suffix(self):
        top = [p for p in tracked('tools') if p.count('/') == 1]
        self.assertTrue(top)
        self.assertEqual([p for p in top if p.endswith('.py')], [])


if __name__ == '__main__':
    unittest.main()
