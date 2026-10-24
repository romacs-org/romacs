"""R218 regression: pinned GNU content, not the bundled-tree denominator."""
import tempfile as _portable_tempfile
from pathlib import Path as _PortablePath
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('census_r218', Path(__file__).with_name('lisp-census.py'))
census = importlib.util.module_from_spec(spec)
spec.loader.exec_module(census)


class DenominatorTests(unittest.TestCase):
    def test_declared_generated_support_is_separate_from_bundled_and_parity(self):
        import json
        import hashlib
        import shutil
        with tempfile.TemporaryDirectory() as tmp:
            root, gnu = Path(tmp) / 'local', Path(tmp) / 'gnu'
            for path in (root, gnu):
                (path / 'lisp').mkdir(parents=True)
                (path / 'lisp/same.el').write_text('nil')
                subprocess.run(['git', 'init', '-q', str(path)], check=True)
                subprocess.run(['git', '-C', str(path), 'add', '.'], check=True)
                subprocess.run(['git', '-C', str(path), '-c', 'user.name=Fixture',
                                '-c', 'user.email=fixture@example.invalid', '-c', 'commit.gpgsign=false',
                                'commit', '-qm', 'fixture'], check=True)
            pin = subprocess.check_output(['git', '-C', str(gnu), 'rev-parse', 'HEAD'], text=True).strip()
            (root / 'tools').mkdir()
            shutil.copyfile(census.ROOT / 'tools/lisp_oracle.py', root / 'tools/lisp_oracle.py')
            support = root / 'lisp/generated.el'
            support.write_text('nil')
            manifest = {'gnu_commit': pin,
                        'sources': {'fixture': {'paths': ['lisp/same.el'], 'recipe': 'fixture'}},
                        'files': [['lisp/generated.el', hashlib.sha256(b'nil').hexdigest(), 'fixture']]}
            (root / 'LISP-GENERATED.json').write_text(json.dumps(manifest))
            with patch.object(census, 'ROOT', root), patch.object(census, 'GNU_SOURCE', gnu), \
                    patch.object(census, 'GNU_PIN', pin):
                inventory = census.oracle_inventory()
                self.assertEqual(inventory['generated_support_paths'], ['lisp/generated.el'])
                self.assertEqual(inventory['bundled_paths'], [])
                rows = [dict(file=name, loaded=True) for name in ('lisp/same.el', 'lisp/generated.el')]
                summary = census.parity_summary(rows, inventory)
                self.assertEqual(summary['gnu_parity']['loaded'], 1)
                self.assertEqual(summary['gnu_parity']['gnu_target_total'], 1)
                self.assertEqual(summary['generated_support'], {'loaded': 1, 'total': 1})
                manifest['files'][0][1] = '0' * 64
                (root / 'LISP-GENERATED.json').write_text(json.dumps(manifest))
                with self.assertRaisesRegex(RuntimeError, 'SUPPORT'):
                    census.oracle_inventory()

    def test_pinned_partition_hashes_and_both_modes(self):
        with tempfile.TemporaryDirectory(dir=str(_PortablePath(_portable_tempfile.gettempdir()) / '')) as tmp:
            root, gnu = Path(tmp) / 'local', Path(tmp) / 'gnu'
            def repo(path, files):
                path.mkdir()
                subprocess.run(['git', 'init', '-q', str(path)], check=True)
                for name, content in files.items():
                    target = path / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text(content)
                subprocess.run(['git', 'add', '.'], cwd=path, check=True)
                subprocess.run(['git', '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture'], cwd=path, check=True)
                return subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=path, text=True).strip()
            pin = repo(gnu, {'lisp/same.el': 'same', 'lisp/drift.el': 'oracle', 'lisp/missing.el': 'missing', 'lisp/failed.el': 'original'})
            repo(root, {'lisp/same.el': 'same', 'lisp/drift.el': 'edited', 'lisp/bundle.el': 'bundle', 'lisp/bundle-failed.el': 'bundle', 'lisp/failed.el': 'edited'})
            # Neither a moving oracle HEAD/worktree nor untracked generated Lisp is authoritative.
            (gnu / 'lisp/same.el').write_text('new oracle HEAD')
            subprocess.run(['git', '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', '-c', 'commit.gpgsign=false', 'commit', '-qam', 'move oracle HEAD'], cwd=gnu, check=True)
            (gnu / 'lisp/same.el').write_text('dirty oracle')
            (root / 'lisp/generated.el').write_text('generated')
            rows = [dict(file='lisp/' + name + '.el', loaded=ok, total_forms=1,
                         forms_evaluated=int(ok), first_failure={'classification': 'unknown'})
                    for name, ok in [('same', True), ('drift', True), ('bundle', True), ('generated', True), ('failed', False), ('bundle-failed', False)]]
            with patch.object(census, 'ROOT', root), patch.object(census, 'GNU_SOURCE', gnu, create=True), patch.object(census, 'GNU_PIN', pin, create=True):
                for mode in ('cold', 'warm'):
                    summary = census.summarize(rows, {r['file'] for r in rows if 'generated' not in r['file']}, mode)
                    self.assertEqual(summary.get('gnu_parity'), {
                        'loaded': 2, 'shared_total': 3, 'gnu_target_total': 4,
                        'loaded_shared_modified': 1, 'shared_modified_total': 2,
                        'missing_total': 1, 'missing_paths': ['lisp/missing.el']})
                    self.assertEqual(summary.get('bundled'), {'loaded': 1, 'total': 2})
                    figure = census.census_figure({'summary': summary, 'mode': mode, 'base_commit': 'fixture', 'oracle_inventory': {'gnu_commit': pin}})
                    for text in (mode.upper(), 'GNU-parity 2/3 shared', 'true GNU target 4', 'bundled 1/2', 'missing 1', 'Loaded shared modified: 1', 'content drift: 2', 'lisp/missing.el', pin):
                        self.assertIn(text, figure)
                inventory = census.oracle_inventory()
            self.assertEqual(inventory['gnu_commit'], pin)
            self.assertEqual(len(inventory['gnu_files']), 4)
            self.assertEqual(inventory['bundled_paths'], ['lisp/bundle-failed.el', 'lisp/bundle.el'])
            by_path = {entry['file']: entry for entry in inventory['gnu_files']}
            import hashlib
            self.assertEqual(by_path['lisp/same.el']['gnu_sha256'], hashlib.sha256(b'same').hexdigest())
            self.assertEqual(by_path['lisp/same.el']['status'], 'pristine')
            self.assertEqual(by_path['lisp/drift.el']['status'], 'modified')
            self.assertEqual(by_path['lisp/missing.el']['status'], 'missing')
            self.assertIsNone(by_path['lisp/missing.el']['local_sha256'])


if __name__ == '__main__':
    unittest.main()
