"""scripts/adapt-modus: the committed rlisp/romacs-theme-modus.el is its
output for the committed modus-themes.el copy."""
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]


class AdaptModusTest(unittest.TestCase):
    def test_the_committed_output_is_current(self):
        run = subprocess.run([sys.executable, '-B', str(ROOT / 'scripts/adapt-modus'), '--check'],
                             capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)


if __name__ == '__main__':
    unittest.main()
