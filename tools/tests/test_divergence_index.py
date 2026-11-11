#!/usr/bin/env python3
"""tools/lib/divergence_index.py: rows read from every prose ledger when asked,
looked up by symbol, checked for attachment, and nothing written.

Run: python3 -m unittest discover -s tools/tests -p test_divergence_index.py -v
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
    def test_a_file_must_hold_the_id_it_is_named_by(self):
        self.assertEqual(self.run_index('--names').returncode, 0)
        (self.dir / 'H5.md').write_text('## H5-SUFFIX\n\nNot H5.\n')
        result = self.run_index('--names')
        self.assertEqual(result.returncode, 1)
        self.assertIn('H5.md: holds no row or section named H5', result.stderr)

    def test_an_id_has_one_file(self):
        (self.dir / 'H9.md').write_text((self.dir / 'H9.md').read_text() + LEDGER.split('\n', 1)[1])
        result = self.run_index('--names')
        self.assertEqual(result.returncode, 1)
        self.assertIn('H1: held by ledger/divergences/H1.md, ledger/divergences/H9.md', result.stderr)

    def test_an_id_without_its_own_file_has_one_file_too(self):
        row = '| C3 | `gamma` | GNU | romacs | deliberate |\n'
        for name in ('H1.md', 'H9.md'):
            (self.dir / name).write_text((self.dir / name).read_text() + row)
        result = self.run_index('--names')
        self.assertEqual(result.returncode, 1)
        self.assertIn('C3: held by ledger/divergences/H1.md, ledger/divergences/H9.md', result.stderr)

    def test_conflict_markers_are_refused(self):
        (self.dir / 'H1.md').write_text('<<<<<<< branch\n' + LEDGER + '=======\n>>>>>>> trunk\n')
        result = self.run_index('--names')
        self.assertEqual(result.returncode, 1)
        self.assertIn('unresolved conflict markers', result.stderr)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='divergence-index-', dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'tools/lib').mkdir(parents=True)
        shutil.copyfile(GENERATOR, self.root / 'tools/lib/divergence_index.py')
        self.dir = self.root / 'ledger' / 'divergences'
        self.dir.mkdir(parents=True)
        (self.dir / 'H1.md').write_text(LEDGER)
        (self.dir / 'H9.md').write_text(LEDGER.replace('H1', 'H9').replace('maphash', 'apply'))
        (self.dir / 'README.md').write_text('# prose\n| X1 | `ignored` | no | no | no |\n')

    def run_index(self, *args):
        return subprocess.run([sys.executable, 'tools/lib/divergence_index.py', *args], cwd=self.root,
                              text=True, capture_output=True)

    def test_a_symbol_finds_its_rows_in_every_file(self):
        result = self.run_index('maphash', 'apply')
        self.assertEqual(result.returncode, 0, result.stderr)
        maphash, apply = map(json.loads, result.stdout.splitlines())
        self.assertEqual([(r['id'], r['source']) for r in maphash], [('H1', 'ledger/divergences/H1.md')])
        self.assertEqual([(r['id'], r['source']) for r in apply], [('H9', 'ledger/divergences/H9.md')])

    def test_counts_and_nothing_written(self):
        before = sorted(p.relative_to(self.root) for p in self.root.rglob('*'))
        result = self.run_index()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('2 rows, 2 symbols, 0 unattached', result.stdout)
        self.assertEqual(before, sorted(p.relative_to(self.root) for p in self.root.rglob('*')))

    def test_a_lookup_does_not_stand_in_for_a_check(self):
        result = self.run_index('--check', 'maphash')
        self.assertEqual(result.returncode, 2)
        self.assertIn('takes no --check', result.stderr)

    def test_check_refuses_a_row_without_a_symbol(self):
        self.assertEqual(self.run_index('--check').returncode, 0)
        (self.dir / 'H2.md').write_text(UNATTACHED)
        result = self.run_index('--check')
        self.assertEqual(result.returncode, 1)
        self.assertIn('Prose only', result.stdout)


if __name__ == '__main__':
    unittest.main()
