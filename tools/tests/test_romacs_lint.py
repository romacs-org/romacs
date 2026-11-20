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
        line = "../../w/tree/lisp/ui/a.el:3: warning: in f: `x' is a free variable; fix: `defvar' it, or bind it"
        self.assertEqual(lint.named_from(line, home, root),
                         "lisp/ui/a.el:3: warning: in f: `x' is a free variable; fix: `defvar' it, or bind it")
        base = "../base-copy/lisp/ui/a.el:5: warning: in f: `x' is a free variable; fix: `defvar' it, or bind it"
        self.assertEqual(lint.named_from(base, '/tmp/home', '/tmp/base-copy').split(':')[0], 'lisp/ui/a.el')
        self.assertEqual(lint.named_from('romacs-lint: 1 file, 0 errors', home, root), 'romacs-lint: 1 file, 0 errors')

    def test_a_file_the_base_lacks_gives_all_its_findings(self):
        self.assertEqual(lint.added([NEW, HINT], []), [NEW, HINT])

    def test_a_file_that_still_does_not_load_is_not_new(self):
        now = 'lisp/a.el:1: error: does not load: Category a is already defined; fix: load it in a batch machine'
        then = 'lisp/a.el:1: error: does not load: (file-missing "romacs-mule"); fix: load it in a batch machine'
        self.assertEqual(lint.added([now], [then]), [])

    def test_a_moved_file_is_compared_with_its_old_path(self):
        status = 'M\0lisp/a.el\0R087\0lisp/b.el\0lisp/ui/b.el\0C100\0lisp/c.el\0lisp/d.el\0D\0lisp/e.el\0'
        self.assertEqual(lint.renames(status), {'lisp/ui/b.el': 'lisp/b.el'})

    def test_the_summary_counts_each_level_and_names_the_base(self):
        self.assertEqual(lint.summary(2, [NEW, HINT], None, 0), 'romacs-lint: 2 files, 1 error, 0 warnings, 1 hint')
        self.assertEqual(lint.summary(1, [], 'abcdef0123456789', 3),
                         'romacs-lint: 1 file, 0 errors, 0 warnings, 0 hints new since abcdef012345 (3 older not shown; --all shows them)')



class JudgedTest(unittest.TestCase):
    """One file the host gives no result for leaves the others judged (romacs#334), and the run incomplete."""
    A, B = 'lisp/session/a.el', 'lisp/session/b.el'
    DIED = 'exit 255'

    def test_a_file_with_no_result_here_and_at_the_base_is_quiet_and_the_others_are_judged(self):
        shown, older, quiet, broken, unbased = lint.judged([self.A, self.B], [[OLD, NEW], self.DIED],
                                                           {self.A: [OLD], self.B: 'timeout'})
        self.assertEqual((shown, older, quiet, broken, unbased), ([NEW], 1, [self.B], [], []))

    def test_a_file_the_base_gives_no_result_for_has_every_finding_new(self):
        shown, older, quiet, broken, unbased = lint.judged([self.A, self.B], [[OLD], [HINT]], {self.A: [OLD], self.B: self.DIED})
        self.assertEqual((shown, older, quiet, broken, unbased), ([HINT], 1, [], [], [self.B]))

    def test_no_result_here_for_a_file_the_base_lints_or_lacks_is_broken(self):
        self.assertEqual(lint.judged([self.A], [self.DIED], {self.A: [OLD]})[3], [self.A])
        self.assertEqual(lint.judged([self.A], [self.DIED], {})[3], [self.A])

    def test_with_no_base_every_file_without_a_result_is_broken(self):
        self.assertEqual(lint.judged([self.A, self.B], [[NEW], self.DIED], None), ([NEW], 0, [], [self.B], []))

    def test_an_unjudged_file_leaves_the_run_incomplete_never_passed(self):
        # The contract (tools/AGENTS.md): a stage that could not run is incomplete, never passed.
        self.assertEqual(lint.verdict([HINT], 1), 2)
        self.assertEqual(lint.verdict([HINT], 0), 0)
        # A real finding still fails, whatever was left unjudged.
        self.assertEqual(lint.verdict([NEW], 1), 1)

    def test_the_summary_counts_the_files_left_unjudged(self):
        self.assertEqual(lint.summary(1, [], 'abcdef0123456789', 2, 1),
                         'romacs-lint: 1 file, 0 errors, 0 warnings, 0 hints new since abcdef012345 '
                         '(2 older not shown; --all shows them); 1 more gave no result, so the lint is incomplete')


if __name__ == '__main__':
    unittest.main()
