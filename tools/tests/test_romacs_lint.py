"""tools/romacs-lint: which findings a change adds, and how they are counted."""
import importlib.machinery
import importlib.util
from pathlib import Path
import unittest

loader = importlib.machinery.SourceFileLoader('romacs_lint', str(Path(__file__).resolve().parents[1] / 'romacs-lint'))
spec = importlib.util.spec_from_loader(loader.name, loader)
lint = importlib.util.module_from_spec(spec)
loader.exec_module(lint)

OLD = "lisp/session/a.el:10: warning: in f: `x' is a free variable; fix: `defvar' it, or bind it"
MOVED = "lisp/session/a.el:42: warning: in f: `x' is a free variable; fix: `defvar' it, or bind it"
NEW = "lisp/session/a.el:50: error: in g: `h' is not defined: fix: require its library"
HINT = "lisp/session/b.el:3: hint: in k: `syntax-ppss': served another way"


class AddedTest(unittest.TestCase):
    def test_only_finding_lines_are_findings(self):
        output = '\n'.join(['Loading a...', OLD, 'romacs-lint: 1 file, 0 errors, 1 warning, 0 hints', NEW])
        self.assertEqual(lint.findings(output), [OLD, NEW])

    def test_a_finding_that_only_moved_is_not_new(self):
        self.assertEqual(lint.added([MOVED, NEW], [OLD]), [NEW])

    def test_a_second_like_finding_is_new(self):
        self.assertEqual(lint.added([OLD, MOVED], [OLD]), [MOVED])

    def test_a_finding_whose_suggestion_changed_is_not_new(self):
        was = "lisp/session/a.el:5: error: in g: `strng-trim' is not defined; fix: did you mean `string-trim'? else require its library"
        now = "lisp/session/a.el:9: error: in g: `strng-trim' is not defined; fix: did you mean `strng-trip'? else require its library"
        self.assertEqual(lint.added([now], [was]), [])

    def test_a_host_that_ends_before_the_closing_line_gave_no_result(self):
        self.assertTrue(lint.COMPLETE.search(NEW + '\nromacs-lint: 1 file, 1 error, 0 warnings, 0 hints\n'))
        self.assertFalse(lint.COMPLETE.search(NEW + '\n'))

    def test_a_findings_file_is_given_from_the_root_whatever_it_was_printed_from(self):
        # The host works in a private home; the judged code, maybe an older lint, prints from there.
        home, root = '/tmp/romacs-lint-x', '/w/tree'
        line = "../../w/tree/lisp/romacs/a.el:3: warning: in f: `x' is a free variable; fix: `defvar' it, or bind it"
        self.assertEqual(lint.named_from(line, home, root),
                         "lisp/romacs/a.el:3: warning: in f: `x' is a free variable; fix: `defvar' it, or bind it")
        base = "../base-copy/lisp/romacs/a.el:5: warning: in f: `x' is a free variable; fix: `defvar' it, or bind it"
        self.assertEqual(lint.named_from(base, '/tmp/home', '/tmp/base-copy').split(':')[0], 'lisp/romacs/a.el')
        self.assertEqual(lint.named_from('romacs-lint: 1 file, 0 errors', home, root), 'romacs-lint: 1 file, 0 errors')

    def test_a_file_the_base_lacks_gives_all_its_findings(self):
        self.assertEqual(lint.added([NEW, HINT], []), [NEW, HINT])

    def test_the_summary_counts_each_level_and_names_the_base(self):
        self.assertEqual(lint.summary(2, [NEW, HINT], None, 0), 'romacs-lint: 2 files, 1 error, 0 warnings, 1 hint')
        self.assertEqual(lint.summary(1, [], 'abcdef0123456789', 3),
                         'romacs-lint: 1 file, 0 errors, 0 warnings, 0 hints new since abcdef012345 (3 older not shown; --all shows them)')


if __name__ == '__main__':
    unittest.main()
