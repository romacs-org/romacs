#!/usr/bin/env python3
"""Behavioral regressions for the real divergence generator, without Cargo.

Run: python3 -m unittest discover -s tools -p test_divergence_index.py -v
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

GENERATOR = Path(__file__).with_name('divergence_index.py')
FIXED_POINT = 'dirty-ledger regenerate/commit/regenerate must preserve the tree and exact content fingerprint'
LEDGER = '## Observable by a package\n| H1 | `maphash` | GNU | romacs | deliberate |\n'


class DivergenceIndexTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='divergence-index-', dir=os.environ.get('TMPDIR', str(GENERATOR.parents[2])))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'tools').mkdir()
        shutil.copyfile(GENERATOR, self.root / 'tools/divergence_index.py')
        (self.root / 'DIVERGENCES.md').write_text(LEDGER)
        for crate in ('z-last', 'a-first'):
            directory = self.root / 'crates' / crate
            directory.mkdir(parents=True)
            (directory / 'GNU-DIVERGENCES.md').write_text(LEDGER)

    def run_command(self, *args):
        result = subprocess.run(args, cwd=self.root, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return result.stdout.strip()

    def generate(self):
        self.run_command(sys.executable, 'tools/divergence_index.py')
        return (self.root / 'DIVERGENCES.json').read_bytes()

    def expected_provenance(self):
        paths = sorted(['DIVERGENCES.md', 'tools/divergence_index.py'] + [p.relative_to(self.root).as_posix() for p in self.root.glob('crates/*/GNU-DIVERGENCES.md')])
        digest = hashlib.sha256()
        for path in paths:
            contents = (self.root / path).read_bytes()
            digest.update(path.encode('utf-8') + b'\0')
            digest.update(str(len(contents)).encode('ascii') + b'\0')
            digest.update(contents)
        return digest.hexdigest(), paths

    def commit(self, message):
        self.run_command('git', 'add', '.')
        self.run_command('git', '-c', 'user.name=Regression', '-c', 'user.email=test@example.invalid', '-c', 'commit.gpgsign=false', 'commit', '-qm', message)

    def test_dirty_ledger_commit_reaches_fixed_point(self):
        self.run_command('git', 'init', '-q')
        self.generate()
        self.commit('baseline')
        ledger = self.root / 'DIVERGENCES.md'
        ledger.write_text(LEDGER.replace('deliberate', 'edited while dirty'))
        first = self.generate()
        self.assertEqual(first, self.generate(), 'two generates with identical inputs must produce identical bytes')
        self.commit('commit changed ledger and regenerated output together')
        committed_tree = self.run_command('git', 'rev-parse', 'HEAD^{tree}')
        final = self.generate()
        self.run_command('git', 'add', '.')
        regenerated_tree = self.run_command('git', 'write-tree')
        provenance = json.loads(final)['provenance']
        fingerprint, paths = self.expected_provenance()
        self.assertTrue(committed_tree == regenerated_tree and provenance.get('input_hash') == fingerprint and provenance.get('inputs') == paths, FIXED_POINT)
        self.assertEqual('', self.run_command('git', 'status', '--porcelain'))

    def test_no_git_exact_sources_and_generator_change_fingerprint(self):
        first = self.generate()
        self.assertEqual(first, self.generate())
        provenance = json.loads(first)['provenance']
        fingerprint, paths = self.expected_provenance()
        self.assertEqual(provenance['input_hash'], fingerprint)
        self.assertEqual(provenance['inputs'], paths)
        self.assertNotIn('describes_commit', provenance)
        self.assertNotIn('describes_date', provenance)
        for relative in ('DIVERGENCES.md', 'crates/a-first/GNU-DIVERGENCES.md', 'tools/divergence_index.py'):
            with self.subTest(relative=relative):
                path = self.root / relative
                path.write_bytes(path.read_bytes() + b'\n# content-only change\n')
                current = json.loads(self.generate())['provenance']['input_hash']
                self.assertNotEqual(fingerprint, current, 'every exact source and generator byte must affect the fingerprint')
                self.assertEqual(current, self.expected_provenance()[0])
                fingerprint = current
        added = self.root / 'crates/new/GNU-DIVERGENCES.md'
        added.parent.mkdir()
        added.write_text(LEDGER)
        self.assertNotEqual(fingerprint, json.loads(self.generate())['provenance']['input_hash'])
        added.unlink()
        self.assertEqual(fingerprint, json.loads(self.generate())['provenance']['input_hash'])

    def test_repairs_stale_provenance_and_noncanonical_bytes(self):
        canonical = self.generate()
        output = self.root / 'DIVERGENCES.json'
        stale = json.loads(canonical)
        stale['provenance'] = {'describes_commit': 'stale', 'describes_date': 'yesterday'}
        output.write_text(json.dumps(stale))
        self.assertEqual(canonical, self.generate(), 'regeneration must heal stale provenance even when answers match')
        output.write_text(json.dumps(json.loads(canonical)))
        self.assertEqual(canonical, self.generate(), 'regeneration must compare complete deterministic bytes')

    def test_check_only_reports_unattached_symbols(self):
        result = self.run_command(sys.executable, 'tools/divergence_index.py', '--check')
        self.assertIn('0 unattached', result)
        output = self.root / 'DIVERGENCES.json'
        self.assertFalse(output.exists())
        (self.root / 'DIVERGENCES.md').write_text('| H2 | no identifier | GNU | romacs | why |\n')
        result = subprocess.run([sys.executable, 'tools/divergence_index.py', '--check'], cwd=self.root, text=True, capture_output=True)
        self.assertEqual(1, result.returncode)
        self.assertIn('1 unattached', result.stdout)
        self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
