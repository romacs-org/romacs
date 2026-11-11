import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from test_vacuity_classifier import receipt_fixture, captured_receipt_fixture

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'test/gnu/tools'))
import member_evidence
import vacuity_binding as binding
sys.path.insert(0, str(ROOT / 'tools/lib'))
import parity_ratchet


class TreeEvidenceLocalTests(unittest.TestCase):
    def test_captured_receipt_reaches_member_consumers_and_refuses_drift(self):
        root, _, _, receipt = captured_receipt_fixture(self)
        path = root.parent / binding.RECEIPT
        path.write_text(json.dumps(receipt))
        self.enterContext(patch.dict(os.environ, ROMACS_VACUITY_RECEIPT=str(path),
                                    ROMACS_MEMBER_EVIDENCE=str(path.parent / binding.MEMBERS)))
        policy = (root / binding.POLICY).read_bytes()
        member_evidence.write(root, path)
        expected = member_evidence.build(root, receipt)['members']
        self.assertEqual(member_evidence.load(root), expected)
        self.assertEqual(parity_ratchet.load_evidence(root), expected)
        spec = importlib.util.spec_from_file_location('captured_gnu_run', ROOT / 'test/gnu/run.py')
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        with patch.object(runner, 'ROOT', root):
            self.assertEqual(runner.load_member_evidence(), expected)
        self.assertEqual((root / binding.POLICY).read_bytes(), policy)
        (root / 'source').write_text('tampered')
        with self.assertRaisesRegex(ValueError, 'tracked bytes'):
            member_evidence.load(root)
        (root / 'source').write_text('candidate')
        receipt['artifacts']['member-results.json'] += 'tampered'
        path.write_text(json.dumps(receipt))
        with self.assertRaisesRegex(ValueError, 'artifact digest'):
            member_evidence.load(root)


class EvidenceLocalTests(unittest.TestCase):
    def setUp(self):
        self.directory = self.enterContext(tempfile.TemporaryDirectory())
        self.run_dir = Path(self.directory)
        self.receipt = self.run_dir / binding.RECEIPT
        self.members = self.run_dir / binding.MEMBERS
        self.data = receipt_fixture()
        self.receipt.write_text(json.dumps(self.data))
        environment = {key: value for key, value in os.environ.items()
                       if key not in ('ROMACS_MEMBER_EVIDENCE', 'ROMACS_VACUITY_RECEIPT',
                                      'ROMACS_VACUITY_GIT_TREE')}
        environment['ROMACS_VACUITY_RECEIPT'] = str(self.receipt)
        self.enterContext(patch.dict(os.environ, environment, clear=True))
        self.enterContext(patch.object(binding, 'projection', return_value=self.data['projection']))
        self.enterContext(patch.object(binding.subprocess, 'check_output', return_value=('2' * 40 + '\n').encode()))
        self.enterContext(patch.object(binding.subprocess, 'run'))

    def test_run_outputs_are_siblings_and_policy_is_not_modified(self):
        policy = (ROOT / binding.POLICY).read_bytes()
        self.assertEqual(member_evidence.write(ROOT, self.receipt), self.members)
        expected = member_evidence.build(ROOT, self.data)['members']
        self.assertEqual(parity_ratchet.load_evidence(ROOT), expected)
        spec = importlib.util.spec_from_file_location('gnu_run_evidence', ROOT / 'test/gnu/run.py')
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        self.assertEqual(runner.load_member_evidence(), expected)
        self.assertEqual((ROOT / binding.POLICY).read_bytes(), policy)

    def test_policy_projection_does_not_require_historical_run_prose(self):
        evidence = member_evidence.build(ROOT, self.data)['members']
        recorded = json.loads((ROOT / 'ledger/parity-ratchet.json').read_text())
        problems = parity_ratchet.regeneration_problems(recorded, evidence)
        self.assertEqual(problems, [])
        key = next(key for key, value in evidence.items()
                   if value['class'] == 'GENUINE' and key in recorded['gnu_suite']['classes'])
        for field, value in (('class', 'VACUOUS'), ('method', 'inspection')):
            mutant = copy.deepcopy(evidence)
            if mutant[key][field] == value:
                value = 'hand'
            mutant[key][field] = value
            self.assertTrue(parity_ratchet.regeneration_problems(recorded, mutant), field)

    def test_accept_gains_retains_every_regression_check(self):
        import contextlib
        import io
        file = 'src/fns-tests.el'
        data = {'gnu_suite': {'files': [file], 'tests_per_file': {file: 10},
                             'passing': {file: ['one', 'two']}},
                'census': {mode: {'loaded': ['a.el']} for mode in ('warm', 'cold')}}
        original = copy.deepcopy(data)
        measurement = {'kind': 'gnu', 'files': [file], 'tests_per_file': {file: 10},
                       'gnu_tests': 10, 'romacs_reported': 3, 'seconds': 0.1,
                       'members': {file: ['one', 'two', 'gain']}, 'verdicts': {}}
        cases = [('gain', measurement, parity_ratchet.EXIT_PASS)]
        lost = copy.deepcopy(measurement)
        lost['members'][file].remove('one')
        cases.append(('loss plus gain', lost, parity_ratchet.EXIT_LOWER))
        empty = copy.deepcopy(measurement)
        empty['romacs_reported'] = 0
        cases.append(('nothing ran', empty, parity_ratchet.EXIT_NOT_MEASURED))
        moved = copy.deepcopy(measurement)
        moved['tests_per_file'][file] = 9
        cases.append(('denominator changed', moved, parity_ratchet.EXIT_NOT_MEASURED))
        budget = copy.deepcopy(measurement)
        budget['classes'] = {f'{file}::{name}': {'class': 'GENUINE', 'counted': True}
                             for name in budget['members'][file]}
        budget['classes'][f'{file}::gain'] = {'class': 'VACUOUS', 'counted': False,
                                            'file_loaded': True, 'why': 'fixture'}
        cases.append(('vacuous budget grew', budget, parity_ratchet.EXIT_LOWER))
        for name, measured, expected in cases:
            with self.subTest(name=name), contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(parity_ratchet.report('fixture', data, measured, accept_gains=True), expected)
        counted_floor = copy.deepcopy(data)
        counted_floor['gnu_suite']['classes'] = {f'{file}::one': {'class': 'GENUINE', 'counted': True}}
        uncounted = copy.deepcopy(measurement)
        uncounted['classes'] = {f'{file}::{name}': {'class': 'UNPROBEABLE', 'counted': False,
                                                  'file_loaded': True, 'why': 'fixture'}
                                for name in uncounted['members'][file]}
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(parity_ratchet.report('fixture', counted_floor, uncounted, accept_gains=True),
                             parity_ratchet.EXIT_LOWER)
        self.assertEqual(data, original, 'accepting gains must not rewrite floors')

    def test_no_fallback_to_obsolete_tracked_evidence(self):
        os.environ.pop('ROMACS_VACUITY_RECEIPT')
        with self.assertRaisesRegex(parity_ratchet.Refusal, 'run-local evidence required'):
            parity_ratchet.load_evidence(ROOT)

    def test_removed_or_forged_subject_cannot_skip_verification(self):
        member_evidence.write(ROOT, self.receipt)
        original = json.loads(self.members.read_text())
        for action in ('remove', 'forge', 'historical'):
            data = copy.deepcopy(original)
            key = 'lisp/emacs-lisp/ert-tests.el::ert-test-should-error-argument'
            if action == 'remove':
                del data['members'][key]
            elif action == 'forge':
                data['members'][key]['manifest_sha256'] = '0' * 64
            else:
                next(value for name, value in data['members'].items() if name != key)['class'] = 'FORGED'
            self.members.write_text(json.dumps(data))
            with self.subTest(action=action), self.assertRaisesRegex(ValueError, 'differs from policy'):
                member_evidence.load(ROOT)

    def test_cross_run_pair_is_refused(self):
        os.environ['ROMACS_MEMBER_EVIDENCE'] = str(self.run_dir / 'different' / binding.MEMBERS)
        with self.assertRaisesRegex(ValueError, 'share a run directory'):
            member_evidence.load(ROOT)

    def test_missing_receipt_is_refused_even_with_member_output(self):
        member_evidence.write(ROOT, self.receipt)
        self.receipt.unlink()
        with self.assertRaises(OSError):
            member_evidence.load(ROOT)

    def test_both_derived_files_are_fingerprinted(self):
        member_evidence.write(ROOT, self.receipt)
        before = parity_ratchet.input_fingerprint(ROOT)
        self.members.write_text(self.members.read_text() + ' ')
        self.assertNotEqual(before, parity_ratchet.input_fingerprint(ROOT))
        member_evidence.write(ROOT, self.receipt)
        self.receipt.write_text(self.receipt.read_text() + ' ')
        self.assertNotEqual(before, parity_ratchet.input_fingerprint(ROOT))

    def test_runtime_receipt_rejects_mutant_source_and_artifacts(self):
        member_evidence.write(ROOT, self.receipt)
        with patch.object(binding, 'projection', return_value='mutant-source'):
            with self.assertRaisesRegex(ValueError, 'source inputs changed'):
                member_evidence.load(ROOT)
        data = copy.deepcopy(self.data)
        data['artifacts']['member-results.json'] += ' '
        self.receipt.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'artifact digest mismatch'):
            member_evidence.load(ROOT)

    def test_policy_cannot_claim_sealed_evidence_or_change_schema(self):
        policy = json.loads((ROOT / binding.POLICY).read_text())
        for mutant in ('schema', 'members', 'sealed', 'subject'):
            data = copy.deepcopy(policy)
            if mutant == 'schema':
                data['schema'] = 999
            elif mutant == 'members':
                data['members'] = []
            elif mutant == 'sealed':
                data['members']['fake'] = {'method': 'sealed-subject-v1', 'class': 'GENUINE'}
            else:
                data['members']['lisp/emacs-lisp/ert-tests.el::ert-test-should-error-argument'] = {}
            with self.subTest(mutant=mutant), patch.object(Path, 'read_text', return_value=json.dumps(data)):
                with self.assertRaisesRegex(ValueError, 'historical member policy'):
                    member_evidence.build(ROOT, self.data)

    def test_policy_contains_only_acceptance_classes_and_methods(self):
        policy = json.loads((ROOT / binding.POLICY).read_text())
        self.assertEqual(set(policy), {'schema', 'members'})
        for value in policy['members'].values():
            self.assertEqual(set(value), {'class', 'method'})
        for field in ('file_failed_forms', 'mutants_killed_in_both', 'why'):
            data = copy.deepcopy(policy)
            next(iter(data['members'].values()))[field] = 1
            with self.subTest(field=field), patch.object(Path, 'read_text', return_value=json.dumps(data)):
                with self.assertRaisesRegex(ValueError, 'only class and method'):
                    member_evidence.build(ROOT, self.data)
        data = dict(policy, provenance={'probe_dir': '/historical/run'})
        with patch.object(Path, 'read_text', return_value=json.dumps(data)):
            with self.assertRaisesRegex(ValueError, 'run observations'):
                member_evidence.build(ROOT, self.data)

    def test_old_output_paths_are_not_source_exemptions(self):
        self.assertEqual(binding.OUTPUTS, frozenset(('ledger/parity-ratchet.json',)))


if __name__ == '__main__':
    unittest.main()
