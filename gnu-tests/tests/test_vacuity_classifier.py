import sys
from pathlib import Path
import unittest
import subprocess
import tempfile
import json
import hashlib
import io
import os
import tarfile
from unittest.mock import patch


sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools'))
import vacuity_classifier as classifier
import vacuity_binding as binding


def receipt_fixture():
    import hashlib
    data = {'classification': 'subject-only-v1', 'observations': classifier.expected_cases()}
    artifacts = {}
    for i, row in enumerate(data['observations']):
        form = classifier.recipes()[row['test']][1 if row['mutation'] == 'original' else 2]
        probe = str(i) + '.json'
        row.update(exit_code=0, artifact_paths=[probe, probe + '.stdout', probe + '.stderr'])
        killed = row['test'] in ('format-c-float', 'ert-test-should-error-argument') and row['mutation'] != 'original'
        outcome = 'failed' if killed else 'passed'
        row['result'] = {'outcome': outcome}
        if row['engine'] == 'romacs-mini':
            artifacts[probe] = json.dumps({'forms': [{'source': form}]})
            artifacts[probe + '.stdout'] = json.dumps({'event': 'result', 'test': row['test'], 'expected': ':passed', 'outcome': outcome}) + '\n' + json.dumps({'event': 'done', 'registered': 1})
        else:
            artifacts[probe] = '\n' + form + '\n'
            artifacts[probe + '.stdout'] = f"VCHAIN|{row['test']}|{outcome}|1|{0 if killed else 1}|{1 if killed else 0}\n"
        artifacts[probe + '.stderr'] = ''
    artifacts['member-results.json'] = json.dumps(data)
    focus = classifier.load_focus()
    manifest = {'source': {'revision': '1' * 40, 'tree': '2' * 40},
                'binaries': {'gnu': {'file': {'sha256': focus.GNU_SHA}, 'provenance': {
                    'kind': 'native-gnu-pin', 'revision': focus.GNU_REV, 'sha256': focus.GNU_SHA}},
                    'romacs': {'file': {'sha256': '3' * 64}, 'provenance': {'kind': 'build-receipt'}}},
                'results': {name: {'sha256': hashlib.sha256(text.encode()).hexdigest(), 'size': len(text.encode())}
                            for name, text in artifacts.items()}}
    text = json.dumps(manifest)
    return {'schema': 1, 'projection': '4' * 64, 'excluded_outputs': sorted(binding.OUTPUTS),
            'manifest': text, 'manifest_sha256': hashlib.sha256(text.encode()).hexdigest(),
            'artifacts': artifacts, 'verdicts': classifier.classify(data)}


def captured_receipt_fixture(test):
    directory = Path(test.enterContext(tempfile.TemporaryDirectory()))
    metadata = directory / 'metadata'
    metadata.mkdir()
    def git(*args):
        return subprocess.check_output(['git', '-C', str(metadata), *args], stderr=subprocess.PIPE).decode().strip()
    git('init', '-q')
    git('config', 'user.name', 'Test')
    git('config', 'user.email', 'test@example.invalid')
    (metadata / 'source').write_text('initial')
    policy = metadata / binding.POLICY
    policy.parent.mkdir(parents=True)
    policy.write_text('{"schema":1,"members":{}}')
    git('add', '.')
    git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'initial')
    (metadata / 'source').write_text('candidate')
    (metadata / 'new-source').write_text('captured new source')
    git('add', '.')
    tree = git('write-tree')
    root = directory / 'captured'
    root.mkdir()
    archive = subprocess.check_output(['git', '-C', str(metadata), 'archive', tree])
    with tarfile.open(fileobj=io.BytesIO(archive)) as files:
        files.extractall(root, filter='data')
    test.enterContext(patch.dict(os.environ, ROMACS_VACUITY_GIT_ROOT=str(metadata),
                                ROMACS_VACUITY_GIT_TREE=tree))
    receipt = receipt_fixture()
    manifest = json.loads(receipt['manifest'])
    manifest['source'] = dict(kind='git-tree', revision=tree, tree=tree)
    receipt['manifest'] = json.dumps(manifest)
    receipt['manifest_sha256'] = hashlib.sha256(receipt['manifest'].encode()).hexdigest()
    receipt['projection'] = binding.projection(root)
    return root, metadata, tree, receipt


class ClassifierTests(unittest.TestCase):
    def observations(self):
        rows = classifier.expected_cases()
        for row in rows:
            killed = row['test'] in ('format-c-float', 'ert-test-should-error-argument')
            row['result'] = {'outcome': 'failed' if killed and row['mutation'] == 'subject-noop' else 'passed'}
        return {'observations': rows}

    def test_controls_and_three_members(self):
        verdicts = classifier.classify(self.observations())
        self.assertEqual(verdicts['ert-test-should-error-argument'], 'ASSERTION-SENSITIVE')
        self.assertEqual(verdicts['ert-test-body-runs'], 'CANNOT-FAIL')
        self.assertEqual(verdicts['ert-test-deftest-lexical-binding-t'], 'UNCLASSIFIED')

    def test_missing_case_refused(self):
        data = self.observations()
        data['observations'].pop()
        with self.assertRaisesRegex(ValueError, 'inventory'):
            classifier.classify(data)

    def test_setup_failure_is_not_kill(self):
        data = self.observations()
        row = next(r for r in data['observations'] if r['test'] == 'ert-test-should-error-argument')
        row.pop('result')
        row['blocked'] = 'registration failure'
        self.assertEqual(classifier.classify(data)['ert-test-should-error-argument'], 'UNCLASSIFIED')

    def test_failed_positive_control_refuses(self):
        data = self.observations()
        for row in data['observations']:
            if row['test'] == 'format-c-float':
                row['result']['outcome'] = 'passed'
        with self.assertRaisesRegex(ValueError, 'positive control'):
            classifier.classify(data)

    def test_failed_negative_control_refuses(self):
        data = self.observations()
        for row in data['observations']:
            if row['test'] == 'vclass-disabled-subject-control':
                row['result']['outcome'] = 'failed'
        with self.assertRaisesRegex(ValueError, 'negative control'):
            classifier.classify(data)

    def test_oracles_retained(self):
        recipes = classifier.recipes()
        original, mutant = recipes['ert-test-should-error-argument'][1:]
        self.assertEqual(mutant, original.replace('(/ 1 0)', 'nil'))
        self.assertIn('(should-error (identity nil))', mutant)
        original, mutant = recipes['vclass-disabled-subject-control'][1:]
        self.assertEqual(original.splitlines()[1], mutant.splitlines()[1])


class BindingTests(unittest.TestCase):
    def test_tree_receipt_replays_and_survives_commit_without_rebinding(self):
        root, metadata, tree, receipt = captured_receipt_fixture(self)
        path = root.parent / binding.RECEIPT
        path.write_text(json.dumps(receipt))
        self.assertEqual(binding.verify_receipt(root, path)[0], receipt)
        subprocess.run(['git', '-C', str(metadata), '-c', 'commit.gpgsign=false',
                        'commit', '-qm', 'candidate'], check=True)
        self.assertEqual(binding.verify_receipt(root, path)[0], receipt)
        (root / 'new-source').write_text('tampered')
        with self.assertRaisesRegex(ValueError, 'tracked bytes'):
            binding.verify_receipt(root, path)

    def test_tree_receipt_requires_matching_authoritative_context(self):
        root, metadata, tree, receipt = captured_receipt_fixture(self)
        path = root.parent / binding.RECEIPT
        path.write_text(json.dumps(receipt))
        initial = subprocess.check_output(['git', '-C', str(metadata), 'rev-parse', 'HEAD^{tree}']).decode().strip()
        head = subprocess.check_output(['git', '-C', str(metadata), 'rev-parse', 'HEAD']).decode().strip()
        for context in (None, initial, head, 'HEAD'):
            with self.subTest(context=context), patch.dict(os.environ):
                if context is None:
                    os.environ.pop('ROMACS_VACUITY_GIT_TREE')
                else:
                    os.environ['ROMACS_VACUITY_GIT_TREE'] = context
                with self.assertRaises((ValueError, subprocess.CalledProcessError)):
                    binding.verify_receipt(root, path)
        manifest = json.loads(receipt['manifest'])
        manifest['source']['kind'] = 'unknown'
        receipt['manifest'] = json.dumps(manifest)
        receipt['manifest_sha256'] = hashlib.sha256(receipt['manifest'].encode()).hexdigest()
        path.write_text(json.dumps(receipt))
        with self.assertRaisesRegex(ValueError, 'regime'):
            binding.verify_receipt(root, path)

    def test_ordinary_commit_receipt_retains_ancestry_requirement(self):
        _, metadata, _, receipt = captured_receipt_fixture(self)
        def git(*args):
            return subprocess.check_output(['git', '-C', str(metadata), *args]).decode().strip()
        head, tree = git('rev-parse', 'HEAD'), git('rev-parse', 'HEAD^{tree}')
        manifest = json.loads(receipt['manifest'])
        manifest['source'] = dict(revision=head, tree=tree)
        receipt['manifest'] = json.dumps(manifest)
        receipt['manifest_sha256'] = hashlib.sha256(receipt['manifest'].encode()).hexdigest()
        receipt['projection'] = binding.projection(metadata, head)
        path = metadata.parent / binding.RECEIPT
        path.write_text(json.dumps(receipt))
        unrelated = git('-c', 'commit.gpgsign=false', 'commit-tree', tree, '-m', 'unrelated')
        git('update-ref', 'refs/heads/unrelated', unrelated)
        git('symbolic-ref', 'HEAD', 'refs/heads/unrelated')
        with self.assertRaises(subprocess.CalledProcessError):
            binding.verify_receipt(metadata, path)
    def test_repeat_record_preserves_only_identical_input_provenance(self):
        import copy
        sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'tools/lib'))
        import parity_ratchet as ratchet
        old = {'payload': [1, 2], 'provenance': {'commit': 'abc', 'tree': 'def',
               'gnu_ert_sha256': '123', 'census_sha256': '456', 'repeat_runs_identical': 2,
               'recorded_at': 'first', 'seconds_total': 1}}
        new = copy.deepcopy(old)
        new['provenance'].update(recorded_at='second', seconds_total=2)
        ratchet.preserve_identical_provenance(old, new)
        self.assertEqual(old, new, 'identical repeated measurements must serialize identically')
        new = copy.deepcopy(old)
        new['payload'].append(3)
        new['provenance']['recorded_at'] = 'changed'
        ratchet.preserve_identical_provenance(old, new)
        self.assertEqual(new['provenance']['recorded_at'], 'changed')
        new = copy.deepcopy(old)
        new['provenance'].update(tree='other', recorded_at='changed')
        ratchet.preserve_identical_provenance(old, new)
        self.assertEqual(new['provenance']['recorded_at'], 'changed')

    def test_external_receipt_replays_and_refuses_tampering(self):
        import copy
        import hashlib
        import os
        from unittest.mock import patch
        root = Path(__file__).resolve().parents[2]
        original = receipt_fixture()
        manifest = json.loads(original['manifest'])
        with tempfile.TemporaryDirectory() as directory:
            external = Path(directory) / 'subject.json'
            environment = {key: value for key, value in os.environ.items()
                           if key != 'ROMACS_VACUITY_GIT_TREE'}
            environment.update(ROMACS_VACUITY_RECEIPT=str(external),
                               ROMACS_MEMBER_EVIDENCE=str(external.parent / binding.MEMBERS))
            with patch.dict(os.environ, environment, clear=True), \
                    patch.object(binding, 'projection', return_value=original['projection']), \
                    patch.object(binding.subprocess, 'check_output', return_value=(manifest['source']['tree'] + '\n').encode()), \
                    patch.object(binding.subprocess, 'run'):
                external.write_text(json.dumps(original))
                self.assertEqual(binding.verify_receipt(root)[0], original)
                for mutation, message in [('result', 'artifact digest'), ('gnu', 'binary provenance'),
                                          ('source', 'source tree')]:
                    forged = copy.deepcopy(original)
                    sealed = json.loads(forged['manifest'])
                    if mutation == 'result':
                        name = next(iter(forged['artifacts']))
                        forged['artifacts'][name] += 'tampered'
                    else:
                        if mutation == 'gnu':
                            sealed['binaries']['gnu']['file']['sha256'] = '0' * 64
                        else:
                            sealed['source']['tree'] = '0' * 40
                        forged['manifest'] = json.dumps(sealed)
                        forged['manifest_sha256'] = hashlib.sha256(forged['manifest'].encode()).hexdigest()
                    external.write_text(json.dumps(forged))
                    with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, message):
                        binding.verify_receipt(root)

    def test_source_drift_and_output_only_successor(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            def git(*args):
                return subprocess.check_output(['git', '-C', directory, *args], stderr=subprocess.PIPE).decode().strip()
            git('init', '-q')
            git('config', 'user.name', 'Test')
            git('config', 'user.email', 'test@example.invalid')
            (root / 'source').write_text('original')
            git('add', '.')
            git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'source')
            original = git('rev-parse', 'HEAD')
            digest = binding.projection(root)
            (root / 'ledger').mkdir()
            (root / 'ledger/parity-ratchet.json').write_text('output')
            git('add', '.')
            git('-c', 'commit.gpgsign=false', 'commit', '-qm', 'output')
            self.assertEqual(digest, binding.projection(root))
            self.assertEqual(digest, binding.projection(root, original))
            (root / 'source').write_text('changed')
            with self.assertRaisesRegex(ValueError, 'tracked bytes'):
                binding.projection(root)
            (root / 'source').write_text('original')
            (root / 'source').chmod(0o755)
            with self.assertRaisesRegex(ValueError, 'mode'):
                binding.projection(root)

    def test_replay_uses_raw_assertion_outcomes_not_claimed_results(self):
        data = {'observations': classifier.expected_cases()}
        artifacts = {}
        for i, row in enumerate(data['observations']):
            form = classifier.recipes()[row['test']][1 if row['mutation'] == 'original' else 2]
            probe = str(i) + '.json'
            row.update(exit_code=0, artifact_paths=[probe, probe + '.stdout', probe + '.stderr'])
            killed = row['test'] in ('format-c-float', 'ert-test-should-error-argument') and row['mutation'] != 'original'
            outcome = 'failed' if killed else 'passed'
            row['result'] = {'outcome': 'passed'}  # deliberately untrusted projection
            if row['engine'] == 'romacs-mini':
                artifacts[probe] = json.dumps({'forms': [{'source': form}]})
                artifacts[probe + '.stdout'] = json.dumps({'event': 'result', 'test': row['test'], 'expected': ':passed', 'outcome': outcome}) + '\n' + json.dumps({'event': 'done', 'registered': 1})
            else:
                artifacts[probe] = '\n' + form + '\n'
                artifacts[probe + '.stdout'] = f"VCHAIN|{row['test']}|{outcome}|1|{0 if killed else 1}|{1 if killed else 0}\n"
            artifacts[probe + '.stderr'] = ''
        class Text:
            def __init__(self, text):
                self.text = text
            def read_text(self):
                return self.text
        def access(_, __, name):
            return Text(artifacts[name])
        self.assertEqual(classifier.replay(data, None, access, {})['ert-test-should-error-argument'], 'ASSERTION-SENSITIVE')
        row = next(row for row in data['observations'] if row['test'] == 'ert-test-should-error-argument' and row['mutation'] != 'original')
        artifacts[row['artifact_paths'][1]] = ''
        self.assertEqual(classifier.replay(data, None, access, {})['ert-test-should-error-argument'], 'UNCLASSIFIED')
        artifacts[row['artifact_paths'][0]] = '{}'
        with self.assertRaises((KeyError, ValueError)):
            classifier.replay(data, None, access, {})

if __name__ == '__main__':
    unittest.main()
