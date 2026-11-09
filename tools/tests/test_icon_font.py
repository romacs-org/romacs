#!/usr/bin/env python3
"""tools/icon_font.py: the checked-in font passes; a font whose sources
changed, or whose glyphs were damaged under an intact record, fails."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import icon_font  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]


class IconFontTest(unittest.TestCase):
    def setUp(self):
        self.data = (ROOT / icon_font.FONT).read_bytes()

    def test_the_checked_in_font_matches(self):
        self.assertIsNone(icon_font.check(ROOT))

    def test_damaged_glyphs_fail(self):
        glyf = self.data.find(icon_font.tables(self.data)[b'glyf'])
        length = len(icon_font.tables(self.data)[b'glyf'])
        damaged = self.data[:glyf] + bytes(length) + self.data[glyf + length:]
        self.assertIn('damaged', icon_font.check_bytes(damaged, ROOT))

    def test_changed_sources_fail(self):
        stale = self.data.replace(icon_font.sources_digest(ROOT).encode('utf-16-be'), ('0' * 64).encode('utf-16-be'))
        self.assertIn('stale', icon_font.check_bytes(stale, ROOT))


if __name__ == '__main__':
    unittest.main()
