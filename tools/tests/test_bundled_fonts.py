#!/usr/bin/env python3
"""tools/lib/bundled_fonts.py: the checked-in fonts pass; a changed file, an
unrecorded font and a README without a file's SHA-256 fail."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import bundled_fonts  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


class BundledFontsTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        fonts = self.root / bundled_fonts.FONTS
        fonts.mkdir(parents=True)
        for path in (ROOT / bundled_fonts.FONTS).iterdir():
            (fonts / path.name).symlink_to(path)
        self.fonts = fonts

    def replace(self, name, data):
        (self.fonts / name).unlink(missing_ok=True)
        (self.fonts / name).write_bytes(data)

    def test_the_checked_in_fonts_match(self):
        self.assertEqual(bundled_fonts.problems(ROOT), [])

    def test_a_changed_font_fails(self):
        self.replace('Iosevka-Bold.ttf', (ROOT / bundled_fonts.FONTS / 'Iosevka-Bold.ttf').read_bytes()[:-1])
        self.assertIn('is not the file', '\n'.join(bundled_fonts.problems(self.root)))

    def test_an_unrecorded_font_fails(self):
        self.replace('Extra.ttf', b'\0')
        self.assertIn('Extra.ttf is in no entry', '\n'.join(bundled_fonts.problems(self.root)))

    def test_a_readme_without_a_files_digest_fails(self):
        readme = (ROOT / bundled_fonts.FONTS / 'README.md').read_text()
        digest = '1ba24b81d82df65a68c9d0bb44bda306fdcdb5d7763afe89b0dea29ea827d1a2'
        self.replace('README.md', readme.replace(digest, '0' * 64).encode())
        self.assertIn('Iosevka-Regular.ttf\'s SHA-256', '\n'.join(bundled_fonts.problems(self.root)))


if __name__ == '__main__':
    unittest.main()
