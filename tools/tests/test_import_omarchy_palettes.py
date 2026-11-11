"""tools/import-omarchy-palettes on a fixture Omarchy themes directory: a
dark palette with a Hyprland-only key and a light one without `orange' and
`brown' make exactly the committed expected files, and --check reports a
changed palette as drift.

python3 -m unittest discover -s tools/tests -p 'test_import_*.py'"""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
FIXTURE = HERE / 'omarchy'
SCRIPT = HERE.parent / 'import-omarchy-palettes'
VERSION = '9.9.9 (fixture)'


def run(*args):
    return subprocess.run([sys.executable, '-B', str(SCRIPT), *map(str, args)], capture_output=True, text=True)


class ImportOmarchyPalettes(unittest.TestCase):
    def setUp(self):
        self.scratch = Path(tempfile.mkdtemp(prefix='omarchy-import-'))
        self.repo = self.scratch / 'repo'
        shutil.copytree(FIXTURE / 'repo', self.repo)
        self.source = self.scratch / 'source'
        shutil.copytree(FIXTURE / 'source', self.source)

    def tearDown(self):
        shutil.rmtree(self.scratch)

    def import_(self, *extra):
        return run('--source', self.source, '--version', VERSION, '--repo', self.repo, *extra)

    def test_makes_the_expected_files(self):
        done = self.import_()
        self.assertEqual(done.returncode, 0, done.stderr)
        expected = FIXTURE / 'expected'
        for path in sorted(p for p in expected.rglob('*') if p.is_file()):
            made = self.repo / path.relative_to(expected)
            self.assertEqual(made.read_bytes(), path.read_bytes(), str(path.relative_to(expected)))

    def test_check_reports_drift_and_nothing_else(self):
        self.assertEqual(self.import_().returncode, 0)
        clean = self.import_('--check')
        self.assertEqual((clean.returncode, clean.stdout), (0, ''), clean.stderr)
        colors = self.source / 'alpha' / 'colors.toml'
        colors.write_text(colors.read_text().replace('#7aa2f7', '#7aa2f8', 1))
        drift = self.import_('--check')
        self.assertEqual(drift.returncode, 1)
        self.assertEqual(drift.stdout.splitlines(), ['drift: etc/themes/omarchy-alpha-theme.el'])
        self.assertIn('#7aa2f7', (self.repo / 'etc/themes/omarchy-alpha-theme.el').read_text())

    def test_refuses_a_bad_colour(self):
        colors = self.source / 'beta' / 'colors.toml'
        colors.write_text(colors.read_text().replace('#d20f39', 'red', 1))
        refused = self.import_()
        self.assertNotEqual(refused.returncode, 0)
        self.assertIn('is not #rrggbb', refused.stderr)


if __name__ == '__main__':
    unittest.main()
