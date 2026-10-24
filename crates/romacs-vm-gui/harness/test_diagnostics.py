"""Later diagnostic-checker qualification, not red-first pixel evidence."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('check_diagnostics', Path(__file__).with_name('check_diagnostics.py'))
assert spec is not None and spec.loader is not None
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class DiagnosticsTests(unittest.TestCase):
    def test_recorded_fixture_and_diagnostic_mutants(self):
        fixture = json.loads((Path(__file__).with_name('fixtures') / 'diagnostic-records.json').read_text())
        with tempfile.TemporaryDirectory(prefix='g1a-diagnostic-fixture-') as temp:
            root = Path(temp)
            for variant, records in fixture.items():
                path = root / variant
                path.mkdir()
                (path / 'binary.stderr').write_text('\n'.join(json.dumps(r) for r in records))
            result = checker.check(root)
        self.assertTrue(result['diagnostic_requirements_hold'])
        self.assertTrue(result['not_pixel_acceptance'])
        for fault in ('unresolved-font', 'frozen-version', 'unmoved-witness'):
            with self.subTest(fault=fault), tempfile.TemporaryDirectory(
                    prefix='g1a-diagnostic-', dir=tempfile.gettempdir()) as temp:
                root = Path(temp)
                data = copy.deepcopy(result['runs'])
                for run in data:
                    if fault == 'unresolved-font':
                        run['paints'][1]['unresolved_face_text_total'] = 1
                    elif fault == 'frozen-version':
                        run['paints'][1]['buffer_version'] = run['paints'][0]['buffer_version']
                    elif run['moving_gc']:
                        run['moving_gc'][0]['witness_moved'] = False
                    path = root / run['variant']
                    path.mkdir()
                    (path / 'binary.stderr').write_text('\n'.join(
                        json.dumps(r) for r in run['paints'] + run['moving_gc']))
                with self.assertRaises(AssertionError):
                    checker.check(root)
