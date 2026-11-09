#!/usr/bin/env python3
"""tools/divergences.py: the split loses no row or line, and a branch that
still edits the old ledgers merges its own entries in, in either order."""
import os
import subprocess
import sys
import tempfile
import unittest
from collections import Counter
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
sys.path.insert(0, str(TOOLS))
import divergence_index  # noqa: E402
import divergences  # noqa: E402

ROOT_LEDGER = """# Deliberate divergences from GNU Emacs

Prose before any section.

## Observable by a package

Notes on this section.

| # | Scope | GNU | Romacs | Why |
|---|---|---|---|---|
| A1 | `alpha` | gnu a | romacs a | why a |
| B2 | `beta` | gnu b | romacs b | why b |
| B2 | `beta` | gnu b | romacs b | why b |
| — | `gamma` | gnu g | romacs g | why g |
| [C3](crates/x/GNU-DIVERGENCES.md#c3) | `delta` | gnu c | romacs c | why c |

| N | GNU | Romacs |  <!-- not-a-divergence -->
|---|---|---|
| 1 | slow | fast |  <!-- not-a-divergence -->
"""
CRATE_LEDGER = """# x: differences

Crate preamble.

## C3 — the long story

More about `delta`.

## Untitled boundary

| # | Scope | GNU | Romacs |
|---|---|---|---|
| D4 | `epsilon` | gnu d | romacs d |
"""


def rows(parsed):
    out = Counter()
    for row in parsed:
        row = dict(row)
        row.pop('source')
        row['id'] = divergences.LINK.sub(r'\1', row['id'] or '') or None
        out[repr(sorted(row.items()))] += 1
    return out


class SplitTest(unittest.TestCase):
    def test_every_row_and_line_survives(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'crates/x').mkdir(parents=True)
            (root / 'DIVERGENCES.md').write_text(ROOT_LEDGER)
            (root / 'crates/x/GNU-DIVERGENCES.md').write_text(CRATE_LEDGER)
            before = rows(divergence_index.parse(root / 'DIVERGENCES.md', 'DIVERGENCES.md')
                          + divergence_index.parse(root / 'crates/x/GNU-DIVERGENCES.md', 'x'))
            divergences.cmd_split(root)
            self.assertFalse((root / 'DIVERGENCES.md').exists())
            after = rows(divergence_index.rows(root))
            self.assertEqual(set(before), set(after))
            self.assertEqual(sum(before.values()) - 1, sum(after.values()), 'only the identical B2 copy collapses')
            names = sorted(p.name for p in (root / divergences.DIR).iterdir())
            self.assertEqual(names, ['A1.md', 'B2.md', 'C3.md', 'D4.md', 'README.md',
                                     'U-observable-by-a-package-gamma.md'])
            text = '\n'.join(p.read_text() for p in (root / divergences.DIR).iterdir())
            for line in (ROOT_LEDGER + CRATE_LEDGER).split('\n'):
                if line.strip() and not line.startswith('# '):
                    self.assertIn(divergences.unlinked(line), text)
            self.assertIn('More about `delta`.', (root / divergences.DIR / 'C3.md').read_text())
            self.assertEqual(divergence_index.misnamed(root), [])


def git(root, *args, check=True):
    return subprocess.run(['git', '-C', str(root), *args], check=check, capture_output=True, text=True)


class MergeTest(unittest.TestCase):
    """trunk splits the ledgers; branches made before still edit them."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='divergences-merge-', dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.tmp.cleanup)
        self.root = root = Path(self.tmp.name)
        git(root, 'init', '-q', '-b', 'trunk')
        for key, value in (('user.name', 'Fixture'), ('user.email', 'fixture@example.invalid'),
                           ('commit.gpgsign', 'false')):
            git(root, 'config', key, value)
        (root / 'crates/x').mkdir(parents=True)
        (root / 'DIVERGENCES.md').write_text(ROOT_LEDGER)
        (root / 'crates/x/GNU-DIVERGENCES.md').write_text(CRATE_LEDGER)
        self.commit('base')
        git(root, 'checkout', '-q', '-b', 'left', 'trunk')
        ledger = root / 'DIVERGENCES.md'
        ledger.write_text(ledger.read_text().replace('romacs a', 'romacs a, edited on left'))
        self.commit('left')
        git(root, 'checkout', '-q', '-b', 'right', 'trunk')
        ledger = root / 'crates/x/GNU-DIVERGENCES.md'
        ledger.write_text(ledger.read_text() + '\nAnd one more line on right.\n')
        self.commit('right')
        self.tips = {b: git(root, 'rev-parse', b).stdout.strip() for b in ('left', 'right')}
        git(root, 'checkout', '-q', 'trunk')
        divergences.cmd_split(root)
        self.commit('split')

    def commit(self, message):
        git(self.root, 'add', '-A')
        git(self.root, 'commit', '-qm', message)

    def merge_trunk_into(self, branch):
        git(self.root, 'checkout', '-q', branch)
        git(self.root, 'merge', '--no-edit', '-X', 'no-renames', 'trunk', check=False)
        status = divergences.cmd_merge(self.root)
        if status == 0:
            git(self.root, 'commit', '-qm', f'merge trunk into {branch}')
        return status

    def land(self, branch):
        self.assertEqual(self.merge_trunk_into(branch), 0)
        git(self.root, 'checkout', '-q', 'trunk')
        git(self.root, 'merge', '-q', '--no-edit', branch)

    def entries(self):
        directory = self.root / divergences.DIR
        return (directory / 'A1.md').read_text(), (directory / 'D4.md').read_text()

    def test_both_branches_land_in_either_order(self):
        start = git(self.root, 'rev-parse', 'trunk').stdout.strip()
        for first, second in (('left', 'right'), ('right', 'left')):
            with self.subTest(first=first):
                git(self.root, 'checkout', '-q', 'trunk')
                git(self.root, 'reset', '-q', '--hard', start)
                for branch, tip in self.tips.items():
                    git(self.root, 'branch', '-q', '-f', branch, tip)
                self.land(first)
                self.land(second)
                a1, d4 = self.entries()
                self.assertIn('romacs a, edited on left', a1)
                self.assertIn('And one more line on right.', d4)
                self.assertFalse((self.root / 'DIVERGENCES.md').exists())
                self.assertEqual(divergence_index.misnamed(self.root), [])

    def test_an_entry_both_sides_changed_is_left_for_a_person(self):
        a1 = self.root / divergences.DIR / 'A1.md'
        a1.write_text(a1.read_text().replace('romacs a', 'romacs a, edited on trunk'))
        self.commit('trunk edits A1')
        self.assertEqual(self.merge_trunk_into('left'), 1)
        text = a1.read_text()
        self.assertIn('<<<<<<< branch', text)
        self.assertIn('edited on left', text)
        self.assertIn('edited on trunk', text)


if __name__ == '__main__':
    unittest.main()
