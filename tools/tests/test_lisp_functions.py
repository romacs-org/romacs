#!/usr/bin/env python3
"""tools/lib/lisp_functions.py: the reader, what a code line is, what declares a
long function, which Lisp is Romacs's own, and the ratchet's findings."""
import json
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import lisp_functions as lf  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]
LAYOUT = {'directories': ['agents', 'ui'], 'off_path': ['vm'], 'packages': 'packages', 'files': []}


def one(text):
    (f,) = lf.functions(text)
    return f


class ReaderTest(unittest.TestCase):
    def test_characters_strings_and_comments_hold_no_parens(self):
        text = '''(defun a ()
  (list ?\\( ?) ?\\" ?\\; ?\\C-\\( ?\\N{LATIN SMALL LETTER A}
        "a ( string ; with \\" quotes )"
        'b) ; a comment (
  )
(defun b () 1)
'''
        self.assertEqual([f.name for f in lf.functions(text)], ['a', 'b'])
        self.assertEqual([n.text for n in lf.read('?\\N{LEFT PARENTHESIS (} x')], ['?\\N{LEFT PARENTHESIS (}', 'x'])
        self.assertEqual([n.text for n in lf.read('(?\\M-; ?\\C-\\M-) ?\\^? ?\\^\\M-; 1)')[0].kids],
                         ['?\\M-;', '?\\C-\\M-)', '?\\^?', '?\\^\\M-;', '1'])

    def test_escaped_symbols_vectors_and_reader_macros(self):
        text = "(defun a () (list foo\\ bar\\( #'car #s(r 1) [x (y)] #x1F ,@z `(,w)))\n"
        self.assertEqual(one(text).name, 'a')

    def test_unbalanced_text_is_an_error(self):
        for text in ('(defun a ()', '(defun a ())]', '(a "b)'):
            with self.assertRaises(lf.ReadError, msg=text):
                lf.functions(text)

    def test_a_definition_inside_another_form_counts_a_nested_one_does_not(self):
        text = "(with-eval-after-load 'x\n  (defun a ()\n    (defun b () 1)))\n'(defun c () 1)\n"
        self.assertEqual([f.name for f in lf.functions(text)], ['a'])

    def test_a_method_is_named_by_its_qualifiers_and_specializers(self):
        text = '(cl-defmethod m :around ((x string) y) "Doc." x)\n(cl-defmethod m ((x integer)) x)\n'
        text += "(cl-defmethod (setf m) (v (x list) &optional z) v)\n"
        text += "(cl-defmethod f (x &context (window-system pgtk) &optional y) x)\n"
        self.assertEqual([f.name for f in lf.functions(text)],
                         ['m :around (string t)', 'm (integer)', '(setf m) (t list)', 'f (t &context (window-system pgtk))'])
        self.assertEqual(one('(cl-defmethod g (x &context ((getenv "A") (eql \'b))) x)').name,
                         'g (t &context ((getenv "A") (eql \'b)))')


class LengthTest(unittest.TestCase):
    def test_docstring_comments_and_blanks_are_not_code(self):
        f = one('(defun a (x)\n  "Doc,\nlong.\n\n(it says)"\n  ;; why\n\n  (car\n   x))\n')
        self.assertEqual((f.lines, f.code, f.data), (9, 3, 0))

    def test_a_defuns_lone_string_is_its_value_not_its_docstring(self):
        self.assertEqual(one('(defun a ()\n  "one\ntwo")\n').code, 3)
        self.assertEqual(one('(define-minor-mode m\n  "Doc\nmore.")\n').code, 2)
        self.assertEqual(one('(cl-defgeneric g (x)\n  "Doc\nmore.")\n').code, 2)

    def test_a_string_in_code_is_code(self):
        f = one('(defun a ()\n  (message "one\ntwo"))\n')
        self.assertEqual(f.code, 3)

    def test_literal_data_is_counted_apart(self):
        f = one("(defun a ()\n  (append '(1\n           2)\n          `(3\n            4)\n          [5\n           6]))\n")
        self.assertEqual((f.code, f.data), (3, 4))

    def test_a_backquote_with_an_unquote_is_code(self):
        f = one('(defmacro a (x)\n  `(progn\n     ,x\n     nil))\n')
        self.assertEqual((f.code, f.data), (4, 0))

    def test_a_modes_docstring_is_found_where_its_definer_puts_it(self):
        minor = one('(define-minor-mode m\n  "Doc\nmore."\n  :global t\n  (ignore))\n')
        derived = one('(define-derived-mode m text-mode "M"\n  "Doc\nmore."\n  (ignore))\n')
        self.assertEqual((minor.code, derived.code), (3, 2))

    def test_a_declaration_is_in_the_comment_lines_directly_above(self):
        self.assertTrue(one(';; SMELL(long-function): a dispatch table\n;; more\n(defun a () 1)\n').declared)
        self.assertFalse(one(';; SMELL(long-function):\n(defun a () 1)\n').declared, 'it gives its reason')
        self.assertFalse(one(';; SMELL(long-function): x\n\n(defun a () 1)\n').declared, 'directly above')
        self.assertFalse(one('(defun a ()\n  ;; SMELL(long-function): x\n  1)\n').declared)


class ScopeTest(unittest.TestCase):
    def test_own_lisp_is_the_subsystems_and_romacs_files_never_a_package(self):
        own = [p for p in ('lisp/agents/x.el', 'lisp/vm/y.el', 'lisp/romacs-z.el', 'lisp/net/romacs-w.el',
                           'lisp/simple.el', 'lisp/net/eww.el', 'lisp/packages/p/romacs-p.el', 'lisp/ui/x.txt',
                           'crates/a/romacs-t.el') if lf.own(LAYOUT, p)]
        self.assertEqual(own, ['lisp/agents/x.el', 'lisp/vm/y.el', 'lisp/romacs-z.el', 'lisp/net/romacs-w.el'])

    def test_a_name_defined_twice_is_kept_twice(self):
        found, errors = lf.census(LAYOUT, [('lisp/ui/a.el', '(defun f () 1)\n'), ('lisp/vm/b.el', '(defun f () 2)\n'),
                                           ('lisp/ui/c.el', '(defun g (')])
        self.assertEqual(sorted(found), ['lisp/ui/a.el f', 'lisp/vm/b.el f'])
        self.assertEqual(len(errors), 1)
        self.assertTrue(errors[0].startswith('lisp/ui/c.el: '))

    def test_every_own_file_of_the_tree_reads(self):
        found, errors = lf.tree_census(ROOT)
        self.assertEqual(errors, [])
        self.assertGreater(len(found), 1000)


class RisesTest(unittest.TestCase):
    def found(self, path='lisp/ui/a.el', **code):
        text = ''.join(f'(defun {name} ()\n' + '  1\n' * (n - 2) + '  1)\n' for name, n in code.items())
        return lf.census(LAYOUT, [(path, text)])[0]

    def test_only_the_undeclared_over_the_limit_are_long(self):
        found = self.found(a=41, b=40)
        found.update(lf.census(LAYOUT, [('lisp/ui/c.el', ';; SMELL(long-function): x\n(defun c ()\n' + ' 1\n' * 90 + ')')])[0])
        self.assertEqual(lf.long_functions(found), {'lisp/ui/a.el a': [41, 'lisp/ui/a.el:1', 'a']})

    def test_a_moved_function_is_itself_and_a_duplicate_name_elsewhere_hides_nothing(self):
        base = lf.long_functions(self.found(f=50))
        moved = lf.long_functions(self.found(path='lisp/vm/b.el', f=51))
        (grown,) = lf.rises(base, moved)
        self.assertTrue(grown.startswith('long_functions: lisp/vm/b.el:1: f grew to 51 code lines from 50'), grown)
        beside = {**lf.long_functions(self.found(path='lisp/agents/early.el', f=2)), **lf.long_functions(self.found(f=51))}
        (grown,) = lf.rises(base, beside)
        self.assertTrue(grown.startswith('long_functions: lisp/ui/a.el:1: f grew to 51'), grown)
        two = {**lf.long_functions(self.found(path='lisp/ui/a.el', f=50)), **lf.long_functions(self.found(path='lisp/vm/b.el', f=44))}
        both_moved = {**lf.long_functions(self.found(path='lisp/ui/c.el', f=44)), **lf.long_functions(self.found(path='lisp/vm/d.el', f=51))}
        (grown,) = lf.rises(two, both_moved)
        self.assertTrue(grown.startswith('long_functions: lisp/vm/d.el:1: f grew to 51 code lines from 50'), grown)
        (grown,) = lf.rises(two, {**lf.long_functions(self.found(path='lisp/ui/a.el', f=44)),
                                  **lf.long_functions(self.found(path='lisp/vm/b.el', f=49))})
        self.assertTrue(grown.startswith('long_functions: lisp/vm/b.el:1: f grew to 49 code lines from 44'), 'one shrinks, one grows')
        base, base_all = self.long_and_all(('lisp/ui/a.el', 60), ('lisp/vm/b.el', 45))
        tree, tree_all = self.long_and_all(('lisp/ui/a.el', 40), ('lisp/ui/c.el', 50))
        (grown,) = lf.rises(base, tree, base_all, tree_all)
        self.assertTrue(grown.startswith('long_functions: lisp/ui/c.el:1: f grew to 50 code lines from 45'),
                        'one drops under the limit where it is, the other moves and grows')

    def long_and_all(self, *places):
        found = {}
        for path, n in places:
            found.update(self.found(path=path, f=n))
        return lf.long_functions(found), found

    def test_a_drop_or_a_shrink_passes_a_growth_or_a_new_one_fails(self):
        base = lf.long_functions(self.found(a=50, b=45))
        self.assertEqual(lf.rises(base, lf.long_functions(self.found(a=49))), [])
        (grown,) = lf.rises(base, lf.long_functions(self.found(a=51, b=45)))
        self.assertTrue(grown.startswith('long_functions: lisp/ui/a.el:1: a grew to 51 code lines from 50; fix: '), grown)
        (new,) = lf.rises(base, lf.long_functions(self.found(a=50, b=45, c=41)))
        self.assertIn('long_functions: 3 over 40 code lines, above the 2', new)
        self.assertIn('new: c (lisp/ui/a.el:96)', new)


class HeldTest(unittest.TestCase):
    def test_long_function_findings_warn_until_enforcing(self):
        found = ['allows: 3, above the 2', 'long_functions: lisp/ui/a.el:1: f grew to 41 code lines from 40']
        with mock.patch.object(lf, 'ENFORCING', False):
            self.assertEqual(lf.held(found), (found[:1], found[1:]))
        with mock.patch.object(lf, 'ENFORCING', True):
            self.assertEqual(lf.held(found), (found, []))


class GitTest(unittest.TestCase):
    def test_a_commit_reads_as_its_working_tree_does(self):
        with tempfile.TemporaryDirectory() as repo:
            def git(*args):
                subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@example.org', '-c',
                                'commit.gpgsign=false', *args], cwd=repo, check=True, stdout=subprocess.DEVNULL)
            (Path(repo) / 'lisp/ui').mkdir(parents=True)
            (Path(repo) / lf.LAYOUT).write_text(json.dumps(LAYOUT))
            (Path(repo) / 'lisp/ui/a.el').write_text('(defun a ()\n  "D\xe9."\n  1)\n')
            (Path(repo) / 'lisp/simple.el').write_text('(defun s () 1)\n')
            git('init', '-q')
            git('add', '-A')
            git('commit', '-q', '-m', 'c')
            self.assertEqual(lf.commit_census(repo, 'HEAD'), lf.tree_census(repo))
            self.assertEqual(list(lf.tree_census(repo)[0]), ['lisp/ui/a.el a'])
            (Path(repo) / lf.LAYOUT).unlink()
            self.assertEqual(lf.tree_census(repo), ({}, []), 'no layout, no own Lisp')


if __name__ == '__main__':
    unittest.main()
