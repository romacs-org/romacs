#!/usr/bin/env python3
"""tools/lib/generated_files.py: a tracked generator output fails the check,
an untracked one passes, and every generator it names exists."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/lib'))
import generated_files  # noqa: E402

OUTPUT = 'out/made.rs'


class GeneratedFilesTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        subprocess.run(['git', 'init', '-q'], cwd=self.root, check=True)
        (self.root / 'out').mkdir()
        (self.root / OUTPUT).write_text('// made\n')

    def test_an_untracked_output_passes(self):
        self.assertEqual(generated_files.tracked(self.root, {OUTPUT: 'build.rs'}), [])

    def test_a_tracked_output_fails(self):
        subprocess.run(['git', 'add', OUTPUT], cwd=self.root, check=True)
        self.assertEqual(generated_files.tracked(self.root, {OUTPUT: 'build.rs'}), [OUTPUT])

    def test_the_tree_tracks_no_output_and_each_generator_exists(self):
        self.assertEqual(generated_files.tracked(ROOT), [])
        for generator in generated_files.OUTPUTS.values():
            self.assertTrue((ROOT / generator.split(',')[0]).is_file(), generator)


if __name__ == '__main__':
    unittest.main()
