#!/usr/bin/env python3
"""Keep Tier B's mutation targets and root-relative evidence citations live."""
import ast
import pathlib
import re
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[4]


class EvidenceTests(unittest.TestCase):
    def test_startup_nil_mutant_reaches_current_predicate(self):
        tree = ast.parse(pathlib.Path(__file__).with_name('startup_mutants.py').read_text())
        mutants = next(ast.literal_eval(node.value) for node in tree.body
                       if isinstance(node, ast.Assign)
                       and any(isinstance(t, ast.Name) and t.id == 'MUTANTS' for t in node.targets))
        nil = next(m for m in mutants if m[0] == 'gnu-nil-counts-as-unset')
        source = (ROOT / 'crates/romacs-vm-gui/src/startup.rs').read_text()
        self.assertEqual(source.count(nil[1]), 1, 'nil mutant must match the current predicate exactly once')
        self.assertNotEqual(nil[1], nil[2])

    def test_divergence_evidence_paths_are_tracked(self):
        rows = [line for path in sorted((ROOT / 'ledger/divergences').glob('*.md'))
                for line in path.read_text().splitlines()]
        refs = [ref for row in rows if row.startswith(('| VM2-G1B |', '| GUI-ML1 |'))
                for ref in re.findall(r'`([^`]+)`', row)
                if ref.endswith(('control-width.txt', 'default-mode-line.txt'))]
        self.assertEqual(len(refs), 2)
        for ref in refs:
            tracked = subprocess.check_output(['git', 'ls-tree', '--name-only', 'HEAD', '--', ref], cwd=ROOT, text=True).strip()
            self.assertEqual(tracked, ref, f'evidence link must resolve in git ls-tree: {ref}')


if __name__ == '__main__':
    unittest.main()
