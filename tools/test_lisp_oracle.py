#!/usr/bin/env python3
"""Focused real-pin oracle controls; all mutations are in disposable copies.

Run: LISP_ORACLE_GNU=/path/to/gnu-emacs python3 tools/test_lisp_oracle.py -v
No commits, indexes, runtime builds or source Lisp writes are performed.
"""
import contextlib
import importlib.util
import json

from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('lisp_oracle', ROOT / 'tools/lisp_oracle.py')
if SPEC is None or SPEC.loader is None:
    raise RuntimeError('cannot load tools/lisp_oracle.py')
ORACLE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ORACLE)
GNU = Path(ORACLE.GNU_SOURCE).resolve()


class RealPinControls(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.pinned = ORACLE.pin_tree(GNU, ORACLE.GNU_PIN)
        cls.manifest = json.loads((ROOT / ORACLE.GENERATED_FILE).read_text())
        cls.scratch = tempfile.TemporaryDirectory(prefix='lisp-oracle-controls-')
        cls.root = Path(cls.scratch.name)
        shutil.copytree(ROOT / 'lisp', cls.root / 'lisp')
        for name in (ORACLE.PIN_FILE, ORACLE.GENERATED_FILE):
            shutil.copyfile(ROOT / name, cls.root / name)
        cls.pristine = next(p for p in sorted(cls.pinned) if p.endswith('.el'))
        cls.support = 'lisp/international/uni-old-name.el'

    @classmethod
    def tearDownClass(cls):
        cls.scratch.cleanup()

    def check(self, root=None):
        result = subprocess.run(
            ['python3', '-B', str(ROOT / 'tools/lisp_oracle.py'), 'check',
             '--root', str(root or self.root), '--gnu', str(GNU)],
            capture_output=True, text=True, check=False)
        return result.returncode, result.stdout + result.stderr

    def refused(self, needle):
        code, output = self.check()
        self.assertEqual(code, 1, output)
        self.assertIn(needle, output)

    @contextlib.contextmanager
    def changed(self, relative, content):
        path = self.root / relative
        before = path.read_bytes() if path.exists() else None
        try:
            if content is None:
                path.unlink()
            else:
                path.write_bytes(content)
            yield
        finally:
            if before is None:
                path.unlink(missing_ok=True)
            else:
                path.write_bytes(before)

    def changed_manifest(self, change):
        record = json.loads((self.root / ORACLE.GENERATED_FILE).read_text())
        change(record)
        return self.changed(ORACLE.GENERATED_FILE, json.dumps(record).encode())

    def test_01_real_repository_and_gitless_snapshot_pass(self):
        self.assertEqual(len(self.pinned), 1627)
        self.assertEqual(len(self.manifest['files']), 81)
        for root in (ROOT, self.root):
            code, output = self.check(root)
            self.assertEqual(code, 0, output)
            self.assertIn('1627 identical, 0 modified', output)
            self.assertIn('81 declared generated support, 0 extra', output)
            print(output.strip())

    def test_02_pinned_byte_edit_refused(self):
        before = (self.root / self.pristine).read_bytes()
        with self.changed(self.pristine, before + b'\n; changed\n'):
            self.refused('MODIFIED ' + self.pristine)

    def test_03_support_byte_edit_refused(self):
        before = (self.root / self.support).read_bytes()
        with self.changed(self.support, before.replace(b'\r\n', b'\n')):
            self.refused('MODIFIED SUPPORT ' + self.support)

    def test_04_undeclared_extra_refused_without_git(self):
        with self.changed('lisp/undeclared-test.el', b'; undeclared\n'):
            self.refused('EXTRA lisp/undeclared-test.el')

    def test_05_missing_pinned_input_refused(self):
        with self.changed(self.pristine, None):
            self.refused('ABSENT ' + self.pristine)

    def test_06_missing_support_input_refused(self):
        with self.changed(self.support, None):
            self.refused('ABSENT SUPPORT ' + self.support)

    def test_07_missing_support_manifest_refused(self):
        with self.changed(ORACLE.GENERATED_FILE, None):
            self.refused(ORACLE.GENERATED_FILE + ' is missing')

    def test_08_support_cannot_override_pristine_paths(self):
        def override(record):
            record['files'].append([self.pristine, '0' * 64, 'autoloads'])
        with self.changed_manifest(override):
            self.refused('support must not override a pristine GNU Git path')

    def test_09_source_reference_must_exist_at_pin(self):
        def bogus(record):
            record['sources']['autoloads']['paths'].append('invented-generator.el')
        with self.changed_manifest(bogus):
            self.refused('source invented-generator.el is not a GNU Git path at the pin')

    def test_10_duplicate_and_unknown_source_refused(self):
        def duplicate(record):
            record['files'].append(record['files'][0])
        with self.changed_manifest(duplicate):
            self.refused('duplicate support path')
        def unknown(record):
            record['files'][0][2] = 'invented-source'
        with self.changed_manifest(unknown):
            self.refused('invalid SHA-256 or unknown source group')

    def test_11_wrong_generated_pin_refused(self):
        def wrong(record):
            record['gnu_commit'] = '0' * 40
        with self.changed_manifest(wrong):
            self.refused('gnu_commit is not the oracle pin')

    def test_12_git_filters_preserve_original_encoded_crlf(self):
        path = ROOT / self.support
        data = path.read_bytes()
        self.assertEqual(len(data), 20035)
        self.assertEqual(data.count(b'\r\n'), 2)
        raw = subprocess.check_output(
            ['git', 'hash-object', '--no-filters', str(path)], cwd=ROOT).strip()
        for autocrlf in ('false', 'true', 'input'):
            filtered = subprocess.check_output(
                ['git', '-c', 'core.autocrlf=' + autocrlf, 'hash-object',
                 '--path=' + self.support, str(path)], cwd=ROOT).strip()
            self.assertEqual(filtered, raw, autocrlf)
        print('uni-old-name.el: 20035 bytes, 2 encoded CRLF pairs; '
              'raw/filtered Git blob ' + raw.decode() + ' for autocrlf=false,true,input')


if __name__ == '__main__':
    unittest.main()
