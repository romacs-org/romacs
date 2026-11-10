"""Census assertions; run with unittest discovery from scripts/."""
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import importlib.machinery
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('census', _source := Path(__file__).resolve().parents[1] / 'lisp-census', loader=importlib.machinery.SourceFileLoader('census', str(_source)))
census = importlib.util.module_from_spec(spec)
try:
    spec.loader.exec_module(census)
except FileNotFoundError:
    census = None

class CensusTests(unittest.TestCase):
    def test_fail_closed(self):
        self.assertIsNotNone(census, 'census implementation missing')
        self.assertFalse(census.loaded({'ok': True, 'steps': [{'status': 'error'}]}))
        self.assertFalse(census.loaded({'ok': False, 'steps': []}))
        self.assertTrue(census.loaded({'ok': True, 'steps': [{'status': 'ok'}]}))

    def test_classification(self):
        self.assertIsNotNone(census, 'census implementation missing')
        for error, kind, symbol in [('void function: defgroup', 'missing_function', 'defgroup'), ('void variable: abc', 'missing_variable', 'abc'), ('nonsense', 'unknown', None), ('reader: invalid escape', 'reader_parse', None)]:
            self.assertEqual(census.classify(error), (kind, symbol))

    def test_handled_dependency_is_not_target_failure(self):
        report = {'ok': True, 'file': '/target.el', 'completed_files': 1, 'steps': [{'file': '/target.el', 'status': 'ok'}, {'file': '/dep.el', 'status': 'error'}]}
        self.assertTrue(census.loaded(report))

    def test_causal_chain_excludes_handled_dependency(self):
        own = {'file': '/target.el', 'ordinal': 1, 'status': 'error', 'error': 'unbound variable'}
        handled = {'file': '/dep.el', 'ordinal': 1, 'status': 'error', 'error': 'void or non-function'}
        self.assertEqual(census.failure_cause([own, handled], own), own)

    def test_cadr_hypothesis_names_existing_source_edit(self):
        rows = [{'file': 'gnu-lisp/example.el', 'first_failure': {'candidate_missing_function': 'cadr', 'cause_file': 'gnu-lisp/emacs-lisp/macroexp.el', 'cause_form_index': 6, 'cause_source': '(defun macroexp--all-forms (forms) nil)'}}]
        import tempfile
        from unittest.mock import patch
        with tempfile.TemporaryDirectory(dir=str(_PortablePath(_portable_tempfile.gettempdir()) / '')) as temporary:
            root = Path(temporary)
            (root / 'gnu-lisp/emacs-lisp').mkdir(parents=True)
            (root / 'gnu-lisp/emacs-lisp/macroexp.el').write_text('\n'.join([''] * 74 + ['(list (cadr var+list))']))
            # Form feeds separate pages, not source lines.
            (root / 'gnu-lisp/subr.el').write_text('\n'.join(['\f'] + [''] * 725 + ['(defun cadr (x)']))
            with patch.object(census, 'ROOT', root):
                result = census.recommendations(rows)[0]
            self.assertIn('(car (cdr var+list))', result['reasoning'])
            self.assertEqual(result['form_index'], 5)

    def test_calibration_prose_tracks_current_measurement(self):
        base = {'file': 'gnu-lisp/subr.el', 'continuation_forms_accepted': 12, 'total_forms': 20, 'strict_load_forms_evaluated': 3, 'recorded_accepted': 440, 'recorded_total': 522, 'gnu_standalone': {'files': [{'status': 'ok'}]}}
        before = census.calibration_note([base])
        changed = dict(base, continuation_forms_accepted=19, strict_load_forms_evaluated=8)
        after = census.calibration_note([changed])
        self.assertIn('continuation 12/20, strict prefix 3/20', before)
        self.assertIn('continuation 19/20, strict prefix 8/20', after)
        self.assertNotEqual(before, after)

    def test_blocking_impact_deduplicates_files(self):
        self.assertIsNotNone(census, 'census implementation missing')
        rows = [{'file': x, 'first_failure': {'classification': 'missing_function', 'symbol': 'foo'}} for x in ['a', 'a', 'b']]
        self.assertEqual(census.rank(rows)[0]['files_blocked'], 2)

if __name__ == '__main__':
    unittest.main()
