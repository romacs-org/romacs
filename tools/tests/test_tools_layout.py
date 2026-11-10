#!/usr/bin/env python3
"""tools/ is the one place for scripts: commands at its top without .py,
Python modules in tools/lib/, tests in tools/tests/; scripts/ holds only
the forwarding stub kept for one release."""
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[2]


def tracked(*paths):
    return subprocess.check_output(['git', '-C', str(ROOT), 'ls-files', *paths], text=True).split()


class ToolsLayout(unittest.TestCase):
    def test_scripts_holds_only_the_forwarding_stub(self):
        self.assertEqual(tracked('scripts'), ['scripts/check-derived'])

    def test_commands_at_the_top_carry_no_py_suffix(self):
        top = [p for p in tracked('tools') if p.count('/') == 1]
        self.assertTrue(top)
        self.assertEqual([p for p in top if p.endswith('.py')], [])


if __name__ == '__main__':
    unittest.main()
