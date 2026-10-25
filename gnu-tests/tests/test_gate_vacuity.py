"""Source/evidence plumbing with fixture observations, not native qualification."""
import contextlib
import copy
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

from test_vacuity_classifier import captured_receipt_fixture
import audit_manifest
import vacuity_binding

TOOLS = Path(__file__).resolve().parents[1] / 'tools'
spec = importlib.util.spec_from_file_location('gate_vacuity_tests', TOOLS / 'gate_vacuity.py')
gate_vacuity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate_vacuity)


class GateVacuityTests(unittest.TestCase):
    def setUp(self):
        self.root, _, self.tree, self.receipt = captured_receipt_fixture(self)
        self.enterContext(patch.object(gate_vacuity, 'ROOT', self.root))
        self.run = self.root.parent / 'runs/one'
        self.run.mkdir(parents=True)
        self.binary = self.root.parent / 'adapter'
        self.binary.write_bytes(b'fixture adapter')
        self.binary.chmod(0o755)
        self.digest = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        (self.run / 'all-binaries.json').write_text(json.dumps({'rust-build-meta': {
            'target-directory': str(self.binary.parent), 'non-test-binaries': {'fixture': [
                dict(name='romacs-gnu-ert', kind='bin-exe', path=self.binary.name)]}}}))
        self.source = audit_manifest._source(self.root)
        manifest = json.loads(self.receipt['manifest'])
        manifest['source'] = self.source
        manifest['binaries']['romacs']['file']['sha256'] = self.digest
        self.receipt['manifest'] = json.dumps(manifest)
        self.receipt['manifest_sha256'] = hashlib.sha256(self.receipt['manifest'].encode()).hexdigest()
        self.cache = self.run.parent.parent / 'subject-cache' / self.tree / (self.digest + '.json')
        self.output = self.run / vacuity_binding.RECEIPT
        self.enterContext(patch.dict(os.environ, ROMACS_MEMBER_EVIDENCE=str(
            self.root.parent / 'previous-run' / vacuity_binding.MEMBERS)))

    def invoke(self, source=None):
        args = ['gate_vacuity', 'prepare', '--run-dir', str(self.run), '--source',
                json.dumps(self.source if source is None else source),
                '--command', json.dumps(['fixture-cargo', 'build'])]
        with patch.object(sys, 'argv', args), contextlib.redirect_stdout(io.StringIO()):
            gate_vacuity.main()

    def test_source_mode_captures_the_submitted_tree(self):
        output = io.StringIO()
        with patch.object(sys, 'argv', ['gate_vacuity', 'source']), contextlib.redirect_stdout(output):
            gate_vacuity.main()
        self.assertEqual(json.loads(output.getvalue()), self.source)
        self.assertEqual(self.source['kind'], 'git-tree')

    def test_prepare_rejects_different_source_before_probing(self):
        changed = dict(self.source, tree='0' * len(self.tree))
        with patch.object(gate_vacuity, 'subprocess') as process, \
                self.assertRaisesRegex(ValueError, 'source changed'):
            self.invoke(changed)
        process.run.assert_not_called()
        self.assertFalse((self.run / 'subject-build.json').exists())

    def test_fresh_evidence_and_build_receipt_bind_tree_kind(self):
        def produce(command, **kwargs):
            if command[2].endswith('import-vacuity-evidence.py'):
                self.output.write_text(json.dumps(self.receipt))
                from member_evidence import write
                write(self.root, self.output)
        with patch.object(gate_vacuity, 'subprocess') as process:
            process.run.side_effect = produce
            self.invoke()
        self.assertEqual(process.run.call_count, 2)
        build = json.loads((self.run / 'subject-build.json').read_text())
        self.assertEqual(build['source'], vacuity_binding.source_identity(self.source))
        self.assertEqual(build['binary_sha256'], self.digest)
        self.assertEqual(json.loads(self.cache.read_text()), self.receipt)
        self.assertTrue((self.run / vacuity_binding.MEMBERS).is_file())
        self.assertEqual(gate_vacuity.os.environ['ROMACS_MEMBER_EVIDENCE'],
                         str(self.run / vacuity_binding.MEMBERS))

    def test_cached_evidence_is_verified_before_member_projection(self):
        self.cache.parent.mkdir(parents=True)
        self.cache.write_text(json.dumps(self.receipt))
        with patch.object(gate_vacuity, 'subprocess') as process:
            self.invoke()
        process.run.assert_not_called()
        self.assertEqual(json.loads(self.output.read_text()), self.receipt)
        self.assertTrue((self.run / vacuity_binding.MEMBERS).is_file())

    def test_cached_wrong_tree_binary_and_artifacts_cannot_become_success(self):
        self.cache.parent.mkdir(parents=True)
        for mutation in ('tree', 'binary', 'artifact'):
            changed = copy.deepcopy(self.receipt)
            manifest = json.loads(changed['manifest'])
            if mutation == 'tree':
                manifest['source']['tree'] = '0' * len(self.tree)
            elif mutation == 'binary':
                manifest['binaries']['romacs']['file']['sha256'] = '0' * 64
            else:
                changed['artifacts']['member-results.json'] += 'tampered'
            changed['manifest'] = json.dumps(manifest)
            changed['manifest_sha256'] = hashlib.sha256(changed['manifest'].encode()).hexdigest()
            self.cache.write_text(json.dumps(changed))
            message = {'tree': 'captured source tree mismatch', 'binary': 'binary differs',
                       'artifact': 'artifact digest'}[mutation]
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, message):
                self.invoke()
            self.assertFalse((self.run / vacuity_binding.MEMBERS).exists())


if __name__ == '__main__':
    unittest.main()
