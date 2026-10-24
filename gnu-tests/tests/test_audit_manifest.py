"""Source-only integration tests; these do not qualify a native audit."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import copy
import hashlib

TOOLS = Path(__file__).resolve().parents[1] / 'tools'
sys.path.insert(0, str(TOOLS))
spec = importlib.util.spec_from_file_location('import_vacuity', TOOLS / 'import-vacuity-evidence.py')
importer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(importer)


class LegacyRefusal(unittest.TestCase):
    def test_unsealed_audit_is_refused_before_reading_results(self):
        with tempfile.TemporaryDirectory() as directory:
            audit = Path(directory)
            (audit / 'vac').mkdir()
            (audit / 'final-verdicts.json').write_text('[]')
            (audit / 'vac/classification.json').write_text('[]')
            (audit / 'file-load-failures.json').write_text('{}')
            (audit / 'final.py').write_text('HAND = {}\nCLEAN_UNPROBED = []\n')
            with self.assertRaisesRegex(ValueError, 'historical HAND audit unsupported; rerun mechanical producer'):
                importer.build(audit)


class ManifestTests(unittest.TestCase):
    def setUp(self):
        import audit_manifest
        self.api = audit_manifest
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / 'repo'
        self.repo.mkdir()
        self.git('init', '-q')
        self.git('config', 'user.email', 'test@example.invalid')
        self.git('config', 'user.name', 'Test')
        (self.repo / 'source').write_text('source\n')
        (self.repo / 'harness.py').write_text('pass\n')
        self.git('add', '.')
        self.git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture')
        self.binary = self.root / 'gnu'
        self.binary.write_bytes(b'native-gnu-test-fixture')
        self.binary.chmod(0o755)
        self.audit = self.root / 'audit'

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.repo), *args]).decode().strip()

    def begin(self):
        import hashlib
        return self.api.begin_run(self.repo, audit_dir=self.audit,
            binaries={'gnu': {'path': str(self.binary), 'provenance': {
                'kind': 'native-gnu-pin', 'revision': 'a' * 40,
                'sha256': hashlib.sha256(self.binary.read_bytes()).hexdigest()}}},
            harness_files=[self.repo / 'harness.py'], commands=[['gnu', '--batch']])

    def seal(self):
        start = self.begin()
        (self.audit / 'result.json').write_text('{}\n')
        self.api.finish_run(start, self.audit, result_files=['result.json'])
        return self.api.verify_manifest(self.audit, repo=self.repo)

    def test_round_trip_binds_exact_source(self):
        manifest = self.seal()
        self.assertEqual(manifest['source']['revision'], self.git('rev-parse', 'HEAD'))
        self.assertEqual(manifest['source']['tree'], self.git('rev-parse', 'HEAD^{tree}'))
        self.assertEqual(manifest['commands'], [['gnu', '--batch']])

    def test_dirty_at_start_refused_even_assume_unchanged(self):
        self.git('update-index', '--assume-unchanged', 'source')
        (self.repo / 'source').write_text('different')
        with self.assertRaisesRegex(ValueError, 'tracked'):
            self.begin()

    def test_dirty_at_end_refused(self):
        start = self.begin()
        (self.audit / 'result.json').write_text('{}')
        (self.repo / 'source').write_text('different')
        with self.assertRaisesRegex(ValueError, 'tracked'):
            self.api.finish_run(start, self.audit, result_files=['result.json'])

    def test_each_bound_input_tamper_refused(self):
        self.seal()
        for path in [self.audit / 'result.json', self.repo / 'harness.py', self.binary]:
            with self.subTest(path=path):
                original = path.read_bytes()
                path.write_bytes(original + b'changed')
                with self.assertRaises(ValueError):
                    self.api.verify_manifest(self.audit, repo=self.repo)
                path.write_bytes(original)

    def test_current_revision_drift_refused(self):
        self.seal()
        self.git('-c', 'commit.gpgsign=false', 'commit', '--allow-empty', '-qm', 'next')
        with self.assertRaisesRegex(ValueError, 'source'):
            self.api.verify_manifest(self.audit, repo=self.repo)

    def test_manifest_tree_forgery_refused(self):
        self.seal()
        path = self.audit / 'audit-manifest.json'
        manifest = json.loads(path.read_text())
        manifest['source']['tree'] = 'b' * 40
        path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, 'source'):
            self.api.verify_manifest(self.audit, repo=self.repo)

    def test_preexisting_results_cannot_be_rebound(self):
        self.audit.mkdir()
        (self.audit / 'old.json').write_text('{}')
        with self.assertRaisesRegex(ValueError, 'empty'):
            self.begin()

    def test_unsealed_result_is_not_consumable(self):
        self.seal()
        (self.audit / 'extra.txt').write_text('old data')
        manifest = self.api.verify_manifest(self.audit, repo=self.repo)
        with self.assertRaisesRegex(ValueError, 'sealed'):
            self.api.require_result(manifest, self.audit, 'extra.txt')

    def test_source_build_receipt_required_not_hash_only(self):
        with self.assertRaisesRegex(ValueError, 'provenance'):
            self.api.begin_run(self.repo, audit_dir=self.audit,
                binaries={'romacs': {'path': str(self.binary)}},
                harness_files=[self.repo / 'harness.py'], commands=[['romacs']])

    def test_focused_import_requires_separate_output(self):
        with self.assertRaises(SystemExit) as error:
            importer.main([str(self.audit), '--format', 'focused'])
        self.assertEqual(error.exception.code, 2)

    def test_result_path_escape_and_symlink_refused(self):
        start = self.begin()
        outside = self.root / 'outside'
        outside.write_text('data')
        for relative in ['../outside', str(outside)]:
            with self.subTest(relative=relative), self.assertRaises(ValueError):
                self.api.finish_run(start, self.audit, result_files=[relative])
        (self.audit / 'linked').symlink_to(outside)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            self.api.finish_run(start, self.audit, result_files=['linked'])

    def test_binary_or_tracked_mode_drift_refused(self):
        self.seal()
        self.binary.chmod(0o644)
        with self.assertRaisesRegex(ValueError, 'digest/mode'):
            self.api.verify_manifest(self.audit)
        self.binary.chmod(0o755)
        (self.repo / 'source').chmod(0o755)
        with self.assertRaisesRegex(ValueError, 'tracked mode'):
            self.api.verify_manifest(self.audit)

    def test_source_build_receipt_binding(self):
        import hashlib
        receipt = self.root / 'build.json'
        receipt.write_text(json.dumps({'schema': 1,
            'source': {'revision': self.git('rev-parse', 'HEAD'),
                       'tree': self.git('rev-parse', 'HEAD^{tree}')},
            'binary_sha256': hashlib.sha256(self.binary.read_bytes()).hexdigest(),
            'command': ['cargo', 'build'], 'exit_code': 0,
            'tracked_clean_start': True, 'tracked_clean_end': True}))
        start = self.api.begin_run(self.repo, audit_dir=self.audit,
            binaries={'romacs': {'path': str(self.binary), 'provenance': {
                'kind': 'build-receipt', 'path': str(receipt)}}},
            harness_files=[self.repo / 'harness.py'], commands=[['romacs']])
        (self.audit / 'result.json').write_text('{}')
        self.api.finish_run(start, self.audit, result_files=['result.json'])
        self.api.verify_manifest(self.audit, repo=self.repo)
        receipt.write_text('{}')
        with self.assertRaises(ValueError):
            self.api.verify_manifest(self.audit, repo=self.repo)


class FocusedImporterTests(ManifestTests):
    def setUp(self):
        super().setUp()
        for name, value in [('ROOT', self.repo), ('GNU_REV', 'a' * 40),
                            ('GNU_SHA', hashlib.sha256(self.binary.read_bytes()).hexdigest())]:
            patcher = patch.object(importer, name, value, create=True)
            patcher.start()
            self.addCleanup(patcher.stop)

    def focused(self, change=None, missing_role=None):
        start = self.begin()
        receipt = self.root / 'build.json'
        receipt.write_text(json.dumps({'schema': 1,
            'source': {k: start['source'][k] for k in ('revision', 'tree')},
            'binary_sha256': hashlib.sha256(self.binary.read_bytes()).hexdigest(),
            'command': ['cargo', 'build'], 'exit_code': 0,
            'tracked_clean_start': True, 'tracked_clean_end': True}))
        start['binaries']['romacs'] = {'file': self.api._file(self.binary),
            'provenance': {'kind': 'build-receipt', 'receipt': self.api._file(receipt)}}
        if missing_role:
            del start['binaries'][missing_role]
        case = {'file': 'fixture.el', 'test': 'example', 'mutation': 'original', 'engine': 'gnu-real'}
        data = {'schema': 1, 'expected_cases': [case], 'observations': [{**case,
            'result': {'test': 'example', 'outcome': 'passed'},
            'artifact_paths': ['case.el', 'case.el.stdout', 'case.el.stderr']}]}
        if change:
            change(data)
        for name in ['case.el', 'case.el.stdout', 'case.el.stderr']:
            (self.audit / name).write_text('fixture artifact')
        (self.audit / 'member-results.json').write_text(json.dumps(data))
        self.api.finish_run(start, self.audit, result_files=[
            'member-results.json', 'case.el', 'case.el.stdout', 'case.el.stderr'])
        return data

    def test_focused_import_round_trip_and_tamper(self):
        expected = self.focused()
        output = self.root / 'projection.json'
        self.assertEqual(importer.main([str(self.audit), '--format', 'focused', '--output', str(output)]), 0)
        self.assertEqual(json.loads(output.read_text())['observations'], expected)
        self.assertEqual(importer.main([str(self.audit), '--format', 'focused', '--output', str(output), '--check']), 0)
        (self.audit / 'case.el.stdout').write_text('tampered')
        with self.assertRaisesRegex(ValueError, 'digest'):
            importer.build_focused(self.audit)

    def test_import_refuses_foreign_manifest_repository(self):
        self.focused()
        foreign = self.root / 'foreign'
        subprocess.check_call(['git', 'clone', '-q', str(self.repo), str(foreign)])
        with patch.object(importer, 'ROOT', foreign), self.assertRaisesRegex(ValueError, 'source'):
            importer.build_focused(self.audit)

    def test_import_requires_both_engine_roles(self):
        self.focused(missing_role='romacs')
        with self.assertRaisesRegex(ValueError, 'roles'):
            importer.build_focused(self.audit)

    def test_import_rejects_self_declared_gnu_pin(self):
        self.focused()
        with patch.object(importer, 'GNU_REV', 'b' * 40), self.assertRaisesRegex(ValueError, 'GNU pin'):
            importer.build_focused(self.audit)
        with patch.object(importer, 'GNU_SHA', 'b' * 64), self.assertRaisesRegex(ValueError, 'GNU pin'):
            importer.build_focused(self.audit)

    def test_invalid_focused_inventories_and_rows_refused(self):
        changes = {
            'schema': lambda d: d.update(schema=2),
            'bool-schema': lambda d: d.update(schema=True),
            'empty': lambda d: d.update(expected_cases=[], observations=[]),
            'missing': lambda d: d.update(observations=[]),
            'extra': lambda d: d['observations'].append({**d['observations'][0], 'test': 'other'}),
            'duplicate-observation': lambda d: d['observations'].append(copy.deepcopy(d['observations'][0])),
            'duplicate-inventory': lambda d: d['expected_cases'].append(copy.deepcopy(d['expected_cases'][0])),
            'wrong-identity': lambda d: d['observations'][0].update(engine='other'),
            'empty-identity': lambda d: d['expected_cases'][0].update(test=''),
            'no-status': lambda d: d['observations'][0].pop('result'),
            'both-statuses': lambda d: d['observations'][0].update(blocked='failed'),
            'empty-result': lambda d: d['observations'][0].update(result={}),
            'bad-blocked': lambda d: (d['observations'][0].pop('result'), d['observations'][0].update(blocked='')),
            'no-artifacts': lambda d: d['observations'][0].pop('artifact_paths'),
            'unsealed-artifact': lambda d: d['observations'][0].update(artifact_paths=['other.el', 'case.el.stdout', 'case.el.stderr']),
            'missing-stderr': lambda d: d['observations'][0].update(artifact_paths=['case.el', 'case.el.stdout']),
            'duplicate-artifact': lambda d: d['observations'][0].update(artifact_paths=['case.el', 'case.el.stdout', 'case.el.stdout']),
            'absolute-artifact': lambda d: d['observations'][0].update(artifact_paths=['/case.el', 'case.el.stdout', 'case.el.stderr']),
        }
        for name, change in changes.items():
            with self.subTest(name=name):
                self.audit = self.root / name
                self.focused(change)
                with self.assertRaises(ValueError):
                    importer.build_focused(self.audit)

    def test_blocked_observation_preserved_without_classification(self):
        expected = self.focused(lambda d: (d['observations'][0].pop('result'),
                                          d['observations'][0].update(blocked='process failed')))
        data = importer.build_focused(self.audit)
        self.assertEqual(data['observations'], expected)
        self.assertNotIn('members', data)

    def test_legacy_sealed_python_never_executed(self):
        start = self.begin()
        marker = self.root / 'executed'
        (self.audit / 'final.py').write_text(f"HAND = {{}}\nopen({str(marker)!r}, 'w').write('bad')\nCLEAN_UNPROBED = []\n")
        (self.audit / 'final-verdicts.json').write_text('[]')
        (self.audit / 'vac').mkdir()
        (self.audit / 'vac/classification.json').write_text('[]')
        (self.audit / 'file-load-failures.json').write_text('{}')
        self.api.finish_run(start, self.audit, result_files=['final.py', 'final-verdicts.json',
            'vac/classification.json', 'file-load-failures.json'])
        with self.assertRaisesRegex(ValueError, 'historical HAND audit unsupported; rerun mechanical producer'):
            importer.build(self.audit)
        self.assertFalse(marker.exists())


if __name__ == '__main__':
    unittest.main()
