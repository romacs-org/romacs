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
    """One file the host gives no result for leaves the others judged (romacs#334)."""
    A, B = 'lisp/session/a.el', 'lisp/session/b.el'
    DIED = 'exit 255'

    def test_a_file_whose_text_the_host_cannot_hold_here_and_at_the_base_is_named_not_judged(self):
        shown, older, quiet, broken, unbased = lint.judged([self.A, self.B], [[OLD, NEW], lint.UNREADABLE],
                                                           {self.A: [OLD], self.B: lint.UNREADABLE})
        self.assertEqual((shown, older, quiet, broken, unbased), ([NEW], 1, [self.B], [], []))

    def test_a_like_failure_that_names_no_limit_of_the_file_is_broken(self):
        # A signal (both runs killed), or the same status with another message, says nothing about the file.
        for cause in ('exit -9', self.DIED):
            self.assertEqual(lint.judged([self.A, self.B], [[OLD], cause], {self.A: [OLD], self.B: cause})[3], [self.B])

    def test_only_the_hosts_text_error_on_a_file_not_utf8_names_its_own_limit(self):
        text_error = 'Loading...\ninvalid Unicode text\n'
        self.assertEqual(lint.cause_of(False, 255, text_error, False), lint.UNREADABLE)
        # The same error while the judged file is UTF-8 came from another file the host read.
        self.assertEqual(lint.cause_of(False, 255, text_error, True), 'exit 255')
        self.assertEqual(lint.cause_of(False, 255, 'Symbol\u2019s value as variable is void: x\n', False), 'exit 255')
        self.assertEqual(lint.cause_of(False, -9, '', False), 'exit -9')
        self.assertIsNone(lint.cause_of(True, 1, '', True))

    def test_a_file_is_utf8_by_its_own_bytes(self):
        import tempfile
        with tempfile.TemporaryDirectory() as directory:
            good, bad = Path(directory) / 'good.el', Path(directory) / 'bad.el'
            good.write_bytes('(defun f () "\u00e9")\n'.encode())
            bad.write_bytes(b'(defconst c "\xf8\x88\x80\x80\x80")\n')  # a 5-byte charset sequence, past Unicode
            self.assertEqual((lint.utf8(good), lint.utf8(bad), lint.utf8(Path(directory) / 'missing.el')), (True, False, False))

    def test_a_timeout_is_never_as_at_the_base(self):
        self.assertEqual(lint.judged([self.A, self.B], [[OLD], 'timeout'], {self.A: [OLD], self.B: 'timeout'})[3], [self.B])
        self.assertEqual(lint.judged([self.A, self.B], [[OLD], 'timeout'], {self.A: [OLD], self.B: self.DIED})[3], [self.B])

    def test_a_failure_of_another_cause_than_the_bases_is_broken(self):
        self.assertEqual(lint.judged([self.A, self.B], [[OLD], 'exit 2'], {self.A: [OLD], self.B: self.DIED})[3], [self.B])

    def test_a_file_the_base_gives_no_result_for_has_every_finding_new(self):
        shown, older, quiet, broken, unbased = lint.judged([self.A, self.B], [[OLD], [HINT]], {self.A: [OLD], self.B: self.DIED})
        self.assertEqual((shown, older, quiet, broken, unbased), ([HINT], 1, [], [], [self.B]))

    def test_no_result_here_for_a_file_the_base_lints_is_broken(self):
        self.assertEqual(lint.judged([self.A], [self.DIED], {self.A: [OLD]})[3], [self.A])

    def test_no_result_for_a_file_the_base_lacks_is_broken(self):
        self.assertEqual(lint.judged([self.A], [self.DIED], {})[3], [self.A])

    def test_with_no_base_every_file_without_a_result_is_broken(self):
        self.assertEqual(lint.judged([self.A, self.B], [[NEW], self.DIED], None), ([NEW], 0, [], [self.B], []))

    def test_when_no_file_has_a_result_the_host_may_be_what_failed(self):
        # A host that cannot start gives no result for every file, here and at the base alike.
        self.assertEqual(lint.judged([self.A, self.B], [self.DIED, self.DIED], {self.A: self.DIED, self.B: self.DIED}),
                         ([], 0, [], [self.A, self.B], []))

    def test_the_summary_names_the_files_left_unjudged(self):
        self.assertEqual(lint.summary(1, [], 'abcdef0123456789', 2, 1),
                         'romacs-lint: 1 file, 0 errors, 0 warnings, 0 hints new since abcdef012345 '
                         '(2 older not shown; --all shows them); 1 more gave no result, as at the base')


if __name__ == '__main__':
    unittest.main()
