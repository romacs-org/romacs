"""R218 regression: pinned GNU content, not the bundled-tree denominator."""
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import importlib.machinery
import json
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('census_r218', _source := Path(__file__).resolve().parents[1] / 'lisp-census', loader=importlib.machinery.SourceFileLoader('census_r218', str(_source)))
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)


class DenominatorTests(unittest.TestCase):
    """The census counts our tree: libraries, generated support, and the bundled files it leaves to
    their suites, from lisp/bundled.json and lisp/GENERATED; no pin and no GNU checkout."""

    def tree(self, tmp):
        root = Path(tmp)
        files = {'lisp/same.el': 'nil', 'lisp/failed.el': 'nil', 'lisp/generated.el': 'nil',
                 'lisp/session/bundle.el': 'nil', 'lisp/romacs-x.el': 'nil',
                 'lisp/packages/dep/bundle-failed.el': 'nil'}
        for name, text in files.items():
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(text)
        (root / 'lisp/bundled.json').write_text(json.dumps(
            {'directories': ['session'], 'packages': 'packages', 'off_path': ['vm'], 'files': []}))
        (root / 'lisp/GENERATED').write_text('# a fixture\nlisp/generated.el\n')
        subprocess.run(['git', 'init', '-q', str(root)], check=True)
        subprocess.run(['git', '-C', str(root), 'add', '.'], check=True)
        return root

    def test_the_population_has_three_parts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.tree(tmp)
            with patch.object(census, 'ROOT', root):
                inventory = census.oracle_inventory()
        self.assertEqual(inventory['library_paths'], ['lisp/failed.el', 'lisp/same.el'])
        self.assertEqual(inventory['generated_support_paths'], ['lisp/generated.el'])
        self.assertEqual(inventory['bundled_paths'],
                         ['lisp/packages/dep/bundle-failed.el', 'lisp/romacs-x.el', 'lisp/session/bundle.el'])

    def test_the_summary_and_figure_count_each_part(self):
        rows = [dict(file=name, loaded=ok, total_forms=1, forms_evaluated=int(ok), first_failure={'classification': 'unknown'})
                for name, ok in [('lisp/same.el', True), ('lisp/failed.el', False), ('lisp/generated.el', True),
                                 ('lisp/session/bundle.el', True), ('lisp/packages/dep/bundle-failed.el', False)]]
        with tempfile.TemporaryDirectory() as tmp:
            root = self.tree(tmp)
            with patch.object(census, 'ROOT', root):
                for mode in ('cold', 'warm'):
                    summary = census.summarize(rows, {r['file'] for r in rows}, mode)
                    self.assertEqual(summary['gnu_parity'], {'loaded': 1, 'shared_total': 2, 'gnu_target_total': 2})
                    self.assertEqual(summary['generated_support'], {'loaded': 1, 'total': 1})
                    self.assertEqual(summary['bundled'], {'loaded': 1, 'total': 3})
                    figure = census.census_figure({'summary': summary, 'mode': mode, 'base_commit': 'fixture'})
                    for text in (mode.upper(), '1/2 libraries load', 'generated support 1/1', 'bundled 1/3'):
                        self.assertIn(text, figure)


if __name__ == '__main__':
    unittest.main()
