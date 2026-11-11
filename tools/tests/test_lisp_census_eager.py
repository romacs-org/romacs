"""Counterfactual artifact identity, comparison and CLI protections."""
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import contextlib
import copy
import importlib.machinery
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('census', _source := Path(__file__).resolve().parents[1] / 'lisp-census', loader=importlib.machinery.SourceFileLoader('census', str(_source)))
assert spec is not None and spec.loader is not None
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)


class EagerArtifactTests(unittest.TestCase):
    def artifacts(self):
        default = {'mode': 'warm', 'scope': 'whole_tree', 'files': [
            {'file': 'a', 'forms_evaluated': 5, 'total_forms': 5, 'loaded': True},
            {'file': 'b', 'forms_evaluated': 1, 'total_forms': 4, 'loaded': False}]}
        eager = copy.deepcopy(default)
        eager.update(eager_target=True, measurement_regime='eager_counterfactual')
        eager['files'][0].update(forms_evaluated=2, loaded=False)
        eager['files'][1].update(forms_evaluated=4, loaded=True)
        return default, eager

    def test_comparison_reports_regressions_even_when_total_delta_is_zero(self):
        result = census.compare_eager(*self.artifacts())
        self.assertEqual(result['loaded_delta'], 0)
        self.assertEqual(result['target_forms_delta'], 0)
        self.assertEqual(result['prefix_regression_count'], 1)
        self.assertEqual(result['prefix_regressions'][0]['file'], 'a')
        self.assertEqual(result['gained_loads'], ['b'])
        self.assertEqual(result['lost_loads'], ['a'])

    def test_comparison_rejects_mixed_inputs_and_duplicate_paths(self):
        for key, value in [('mode', 'cold'), ('input_sha256', 'other'), ('bootstrap_chain', [])]:
            default, eager = self.artifacts()
            eager[key] = value
            with self.assertRaises(ValueError):
                census.compare_eager(default, eager)
        default, eager = self.artifacts()
        eager['files'].append(eager['files'][0])
        with self.assertRaises(ValueError):
            census.compare_eager(default, eager)

    def test_unavailable_progress_is_not_a_zero_delta(self):
        default, eager = self.artifacts()
        eager['files'][0]['forms_evaluated'] = None
        result = census.compare_eager(default, eager)
        self.assertIsNone(result['target_forms_delta'])
        self.assertEqual(result['unavailable_prefix_comparisons'], ['a'])

    def test_freshness_check_requires_matching_flag(self):
        with tempfile.TemporaryDirectory(dir=str(_PortablePath(_portable_tempfile.gettempdir()) / '')) as tmp:
            root = Path(tmp)
            (root / 'lisp/gnu').mkdir()
            artifact = root / 'measurement.json'
            data = {'mode': 'warm', 'oracle_inventory': {}, 'input_sha256': 'same', 'scope': 'whole_tree', 'files': [], 'gnu_version': 'GNU test'}
            with patch.object(census, 'ROOT', root), patch.object(census, 'oracle_inventory', return_value={}), patch.object(census, 'fingerprint', return_value='same'), patch.object(census, 'gnu_oracle_binary', return_value='/unit-test-gnu'), patch.object(census.subprocess, 'check_output', return_value='GNU test\n'), contextlib.redirect_stdout(io.StringIO()):
                for flag in [False, True]:
                    data['eager_target'] = flag
                    artifact.write_text(json.dumps(data))
                    for requested in [False, True]:
                        argv = ['census', '--check', '--mode', 'warm', '--output', str(artifact)] + (['--eager-target'] if requested else [])
                        with patch('sys.argv', argv):
                            self.assertEqual(census.main(), 0 if flag == requested else 1)

    def test_eager_cannot_overwrite_ordinary_artifact(self):
        with patch('sys.argv', ['census', '--eager-target', '--output', str(census.ARTIFACT)]), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as failure:
                census.main()
            self.assertEqual(failure.exception.code, 2)


if __name__ == '__main__':
    unittest.main()
