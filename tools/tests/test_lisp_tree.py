import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import lisp_tree

MAP = lisp_tree.load_map('''# kind\tfrom\tto
file\tlisp/romacs/term/romacs-preload.el\tlisp/ui/romacs-preload.el
dir\tlisp/romacs/gnosis/\tlisp/packages/gnosis/
dir\tlisp/romacs/vm/\tlisp/vm/
dir\tlisp/gnu/\tlisp/
gone\tlisp/romacs/hermes/\tdeleted by another lane
rule\tromacs-agent*.el\tlisp/agents/
rule\tromacs-dired.el\tlisp/
''')

FILES = {
    'lisp/gnu/subr.el': ';; GNU\n',
    'lisp/gnu/emacs-lisp/pcase.el': ';; GNU\n',
    'lisp/romacs/romacs-agent.el': '(load "lisp/romacs/romacs-dired")\n',
    'lisp/romacs/romacs-dired.el': ';; ours\n',
    'lisp/romacs/vm/prelude.el': ';; vm\n',
    'lisp/romacs/gnosis/gnosis.el': ';; package, own text: lisp/romacs/vm/\n',
    'lisp/romacs/term/romacs-preload.el': ';; preload\n',
    'tools/tests/fixture/repo/lisp/romacs/romacs-dired.el': ';; a fixture\n',
    'tools/x.py': ("ROOT / 'lisp/gnu' / 'emacs-lisp'\n'lisp/gnu/subr.el', 'lisp/romacs/romacs-agent.el'.\n"
                   "'lisp/gnu-tests' 'lisp/romacs/vm/prelude.el' 'lisp/romacs/'\n"),
}


class Placement(unittest.TestCase):
    def test_rows_rules_and_directories(self):
        self.assertEqual(lisp_tree.target('lisp/romacs/romacs-agent-ui.el', MAP), 'lisp/agents/romacs-agent-ui.el')
        self.assertEqual(lisp_tree.target('lisp/romacs/romacs-dired.el', MAP), 'lisp/romacs-dired.el')
        self.assertEqual(lisp_tree.target('lisp/gnu/emacs-lisp/pcase.el', MAP), 'lisp/emacs-lisp/pcase.el')
        self.assertEqual(lisp_tree.target('lisp/romacs/term/romacs-preload.el', MAP), 'lisp/ui/romacs-preload.el')
        self.assertEqual(lisp_tree.placed('t/repo/lisp/romacs/romacs-dired.el', MAP), 't/repo/lisp/romacs-dired.el')

    def test_unplaced_and_gone_are_refused(self):
        with self.assertRaisesRegex(lisp_tree.Unplaced, 'place lisp/romacs/romacs-new.el'):
            lisp_tree.target('lisp/romacs/romacs-new.el', MAP)
        with self.assertRaisesRegex(lisp_tree.Unplaced, 'deleted by another lane'):
            lisp_tree.target('lisp/romacs/hermes/hermes.el', MAP)

    def test_references_follow_and_the_unplaceable_are_listed(self):
        text, left = lisp_tree.rewrite(FILES['tools/x.py'], MAP)
        self.assertEqual(text, ("ROOT / 'lisp' / 'emacs-lisp'\n'lisp/subr.el', 'lisp/agents/romacs-agent.el'.\n"
                                "'lisp/gnu-tests' 'lisp/vm/prelude.el' 'lisp/romacs/'\n"))
        self.assertEqual(left, ['lisp/romacs/'])


class Move(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.root = Path(self.scratch.name)
        for path, text in FILES.items():
            (self.root / path).parent.mkdir(parents=True, exist_ok=True)
            (self.root / path).write_text(text)
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        subprocess.run(['git', '-C', str(self.root), 'add', '-A'], check=True)

    def tearDown(self):
        self.scratch.cleanup()

    def read(self, path):
        return (self.root / path).read_text()

    def test_move_then_relink_is_whole_and_idempotent(self):
        lisp_tree.move(self.root, MAP)
        self.assertFalse((self.root / 'lisp/gnu').exists() or (self.root / 'lisp/romacs').exists())
        self.assertEqual(self.read('lisp/emacs-lisp/pcase.el'), ';; GNU\n')
        self.assertEqual(self.read('lisp/packages/gnosis/gnosis.el'), FILES['lisp/romacs/gnosis/gnosis.el'])
        self.assertTrue((self.root / 'tools/tests/fixture/repo/lisp/romacs-dired.el').is_file())
        self.assertFalse((self.root / 'tools/tests/fixture/repo/lisp/romacs').exists())
        self.assertEqual(lisp_tree.move(self.root, MAP), [])
        changes, unplaced = lisp_tree.plan(self.root, MAP)
        self.assertEqual(changes['lisp/agents/romacs-agent.el'], '(load "lisp/romacs-dired")\n')
        self.assertNotIn('lisp/packages/gnosis/gnosis.el', changes, 'a package keeps its own text')
        self.assertEqual(unplaced, {'tools/x.py': ['lisp/romacs/']})

    def test_a_branch_file_no_rule_places_stops_the_move(self):
        (self.root / 'lisp/romacs/romacs-new.el').write_text(';; new\n')
        subprocess.run(['git', '-C', str(self.root), 'add', '-A'], check=True)
        with self.assertRaisesRegex(SystemExit, 'place lisp/romacs/romacs-new.el'):
            lisp_tree.move(self.root, MAP)
        self.assertTrue((self.root / 'lisp/gnu/subr.el').is_file(), 'nothing moved')


if __name__ == '__main__':
    unittest.main()
