#!/usr/bin/env python3
"""tools/lib/divergence_index.py: rows read from every prose ledger when asked,
looked up by symbol, checked for attachment, and nothing written.

Run: python3 -m unittest discover -s tools -p test_divergence_index.py -v
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

GENERATOR = Path(__file__).resolve().parents[1] / 'lib/divergence_index.py'
LEDGER = '## Observable by a package\n| H1 | `maphash` | GNU | romacs | deliberate |\n'
UNATTACHED = '## Observable by a package\n| H2 | Prose only | GNU | romacs | deliberate |\n'


class DivergenceIndexTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='divergence-index-', dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'tools/lib').mkdir(parents=True)
        shutil.copyfile(GENERATOR, self.root / 'tools/lib/divergence_index.py')
        (self.root / 'DIVERGENCES.md').write_text(LEDGER)
        directory = self.root / 'crates' / 'a-crate'
        directory.mkdir(parents=True)
        (directory / 'GNU-DIVERGENCES.md').write_text(LEDGER.replace('maphash', 'apply'))

    def run_index(self, *args):
        return subprocess.run([sys.executable, 'tools/lib/divergence_index.py', *args], cwd=self.root,
                              text=True, capture_output=True)

    def test_a_symbol_finds_its_rows_in_every_ledger(self):
        result = self.run_index('maphash', 'apply')
        self.assertEqual(result.returncode, 0, result.stderr)
        maphash, apply = map(json.loads, result.stdout.splitlines())
        self.assertEqual([(r['id'], r['source']) for r in maphash], [('H1', 'DIVERGENCES.md')])
        self.assertEqual([(r['id'], r['source']) for r in apply], [('H1', 'crates/a-crate/GNU-DIVERGENCES.md')])

    def test_counts_and_nothing_written(self):
        before = sorted(p.relative_to(self.root) for p in self.root.rglob('*'))
        result = self.run_index()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('2 rows, 2 symbols, 0 unattached', result.stdout)
        self.assertEqual(before, sorted(p.relative_to(self.root) for p in self.root.rglob('*')))

    def test_check_refuses_a_row_without_a_symbol(self):
        self.assertEqual(self.run_index('--check').returncode, 0)
        (self.root / 'DIVERGENCES.md').write_text(LEDGER + UNATTACHED.split('\n', 1)[1])
        result = self.run_index('--check')
        self.assertEqual(result.returncode, 1)
        self.assertIn('Prose only', result.stdout)


if __name__ == '__main__':
    unittest.main()
