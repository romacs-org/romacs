#!/usr/bin/env python3
"""scripts/dump-vm-bench: lexical source fixture, GNU oracle and diagnostic arm."""
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from importlib.machinery import SourceFileLoader
from importlib.util import module_from_spec, spec_from_loader


def load_script(path, name):
    loader = SourceFileLoader(name, str(path))
    spec = spec_from_loader(name, loader)
    assert spec is not None
    module = module_from_spec(spec)
    loader.exec_module(module)
    return module

dump = load_script(Path(__file__).with_name('dump-vm-bench'), 'dump_vm_bench')
GNU = shutil.which('emacs') or '/usr/bin/emacs'
HAVE_GNU = Path(GNU).is_file()

# No lexical-binding cookie anywhere: the fixture consumer contract is lexical.
SYNTHETIC = r''';;; synthetic.el --- no binding cookie on purpose
;; A comment with (unbalanced parens and a "string
(defvar core-bench--all nil "Rows; docstring with ( and ; inside.")
(defmacro core-bench--def (name &rest body)
  `(progn (defun ,(intern (format "core-bench-row-%s" name)) () ,@body)
          (push (cons ',name ',(intern (format "core-bench-row-%s" name))) core-bench--all)))
(defun core-bench--helper (n) "Doc ; ( )" (* n 2))
(defun core-bench--unused () 'never-exported)
(core-bench--def lex
  ;; Lexical: the closure sees the inner x=2. Dynamic binding would see x=1.
  (let ((x 1)) (funcall (let ((x 2)) (lambda () x)))))
(core-bench--def twice (core-bench--helper 21))
'''
CHECK = r'''(defun core-bench--check (name result)
  (unless (equal result (cdr (assoc name '(("lex" . 2) ("twice" . 42)))))
    (error "Wrong answer for %s" name)))
'''
WANTED = [('defvar', 'core-bench--all'), ('defmacro', 'core-bench--def'),
          ('defun', 'core-bench--helper'), ('core-bench--def', 'lex'), ('core-bench--def', 'twice')]


@unittest.skipUnless(HAVE_GNU, 'GNU Emacs oracle required')
class SourceFixtureTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.bench = self.root / 'synthetic.el'
        self.bench.write_text(SYNTHETIC)
        self.check = self.root / 'check.el'
        self.check.write_text(CHECK)

    def work(self, name):
        path = self.root / name
        path.mkdir()
        return path

    def test_forms_are_verbatim_slices_in_file_order(self):
        forms = dump.extract_forms(self.bench, list(reversed(WANTED)), self.work('extract'), GNU)
        self.assertEqual([(f['operator'], f['name']) for f in forms], WANTED)
        for form in forms:
            self.assertEqual(SYNTHETIC.count(form['text']), 1)
            self.assertTrue(form['text'].startswith('(' + form['operator']))
        self.assertNotIn('core-bench--unused', json.dumps(forms))

    def test_missing_source_form_is_refused(self):
        with self.assertRaisesRegex(ValueError, 'missing'):
            dump.extract_forms(self.bench, WANTED + [('defun', 'core-bench--absent')], self.work('missing'), GNU)

    def test_source_consumer_control_evaluates_only_fixture_forms_lexically(self):
        data = dump.source_fixture(self.bench, ['lex', 'twice'], WANTED, self.work('fixture'), self.check, GNU)
        self.assertEqual((data['schema'], data['input']), (2, 'elisp-source'))
        self.assertEqual({row: value['expected'] for row, value in data['rows'].items()}, {'lex': 2, 'twice': 42})
        self.assertIn('interpreted', data['oracle']['method'])
        text = json.dumps(data)
        self.assertNotIn('functions', data)
        for struck in ('"closure":', 'byte-code', '.elc', 'lexical-binding'):
            self.assertNotIn(struck, text)

    def test_tampered_source_form_fails_the_independent_check(self):
        tampered = self.root / 'tampered.el'
        tampered.write_text(SYNTHETIC.replace('(* n 2)', '(* n 3)'))
        with self.assertRaisesRegex(ValueError, 'Wrong answer for twice'):
            dump.source_fixture(tampered, ['lex', 'twice'], WANTED, self.work('tampered'), self.check, GNU)

    def test_oracle_refuses_a_compiled_row_entry(self):
        data = dump.source_fixture(self.bench, ['twice'], WANTED, self.work('ok'), None, GNU)
        data['source']['forms'].append({'operator': 'byte-compile', 'name': 'x',
                                        'text': "(byte-compile 'core-bench-row-twice)"})
        path = self.root / 'compiled.json'
        path.write_text(json.dumps(data))
        with self.assertRaisesRegex(ValueError, 'not an interpreted source closure'):
            dump.source_oracle(path, ['twice'], self.work('compiled'), None, GNU)


@unittest.skipUnless(HAVE_GNU, 'GNU Emacs oracle required')
class CoreBenchFixtureTests(unittest.TestCase):
    """The real V0 rows: 15 GNU-checked answers from source forms only."""

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        root = Path(cls.tmp.name)
        cls.source = root / 'source.json'
        cls.bytecode = root / 'bytecode.json'
        dump.generate(cls.source, GNU)
        dump.generate(cls.bytecode, GNU, diagnostic=True)
        cls.data = json.loads(cls.source.read_text())
        cls.diagnostic = json.loads(cls.bytecode.read_text())

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_every_row_has_a_checked_answer_from_lexical_source(self):
        self.assertEqual(list(self.data['rows']), dump.ROWS)
        self.assertEqual(len(dump.ROWS), 15)
        self.assertEqual((self.data['schema'], self.data['input']), (2, 'elisp-source'))
        self.assertIn('core-bench--check', self.data['oracle']['method'])
        names = {(f['operator'], f['name']) for f in self.data['source']['forms']}
        self.assertEqual(names, set(dump.SOURCE_FORMS))
        self.assertNotIn('functions', self.data)

    def test_source_answers_equal_the_bytecode_diagnostic_answers(self):
        for row in dump.ROWS:
            with self.subTest(row=row):
                self.assertEqual(self.data['rows'][row]['expected'], self.diagnostic['rows'][row]['expected'])

    def test_bytecode_arm_is_labelled_diagnostic_never_input(self):
        self.assertEqual(self.diagnostic['input'], 'gnu-bytecode-diagnostic')
        self.assertIn('NOT an Elisp input', self.diagnostic['diagnostic'])
        self.assertIn('core-bench-row-bc-closure-counter', self.diagnostic['functions'])


if __name__ == '__main__':
    unittest.main()
