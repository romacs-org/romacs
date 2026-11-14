#!/usr/bin/env python3
"""tools/lib/lisp_tree.py: the trees move under lisp/, every reference follows
except GNU's text, the vendored packages and the options, and a branch that
merges the move catches up by rerunning.

Run: python3 -m unittest discover -s tools/tests -p test_lisp_tree.py -v
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

TOOL = Path(__file__).resolve().parents[1] / 'lib/lisp_tree.py'
FILES = {
    'gnu-lisp/a.el': ';; GNU text that says rlisp stays\n',
    'rlisp/b.el': '(load "gnu-lisp/a")\n',
    'rlisp/deps/pkg/pkg.el': ';; vendored: rlisp stays\n',
    'tests/fixture/repo/rlisp/theme.el': ';; a fixture mirroring the layout\n',
    'src/main.rs': ('const A: &str = include_str!("../gnu-lisp/a.el");\n'
                    'const ARGS: [&str; 2] = ["--gnu-lisp", "--no-gnu-lisp"];\n'
                    'fn owned_modules_live_in_rlisp() {}\n'
                    'const IGNORE: &str = "gnu-lisp/x.elc\\ngnu-lisp/y.elc\\n\\trlisp/z.elc";\n'),
}


def git(root, *args):
    return subprocess.run(['git', '-C', str(root), *args], check=True, capture_output=True, text=True).stdout


def tool(root, *args):
    return subprocess.run([sys.executable, '-B', str(TOOL), *args, str(root)], capture_output=True, text=True)


class LispTreeTest(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix='lisp-tree-', dir=os.environ.get('TMPDIR'))
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        git(self.root, 'init', '-q', '-b', 'trunk')
        for key, value in (('user.name', 'Fixture'), ('user.email', 'fixture@example.invalid'),
                           ('commit.gpgsign', 'false')):
            git(self.root, 'config', key, value)
        for path, text in FILES.items():
            (self.root / path).parent.mkdir(parents=True, exist_ok=True)
            (self.root / path).write_text(text)
        git(self.root, 'add', '-A')
        git(self.root, 'commit', '-qm', 'base')

    def read(self, path):
        return (self.root / path).read_text()

    def run_all(self):
        for command in ('move', 'relink'):
            self.assertEqual(tool(self.root, command).returncode, 0)
        return tool(self.root, 'check')

    def test_the_trees_move_and_the_references_follow(self):
        self.assertEqual(tool(self.root, 'check').returncode, 1)
        check = self.run_all()
        self.assertEqual(self.read('lisp/gnu/a.el'), FILES['gnu-lisp/a.el'])
        self.assertEqual(self.read('lisp/romacs/b.el'), '(load "lisp/gnu/a")\n')
        self.assertEqual(self.read('lisp/romacs/deps/pkg/pkg.el'), FILES['rlisp/deps/pkg/pkg.el'])
        self.assertEqual(self.read('tests/fixture/repo/lisp/romacs/theme.el'), FILES['tests/fixture/repo/rlisp/theme.el'])
        self.assertFalse((self.root / 'tests/fixture/repo/rlisp').exists())
        self.assertEqual(self.read('src/main.rs'), FILES['src/main.rs'].replace(
            '../gnu-lisp/a.el', '../lisp/gnu/a.el').replace('"gnu-lisp/x.elc\\ngnu-lisp/y.elc\\n\\trlisp/z.elc',
                                                            '"lisp/gnu/x.elc\\nlisp/gnu/y.elc\\n\\tlisp/romacs/z.elc'))
        self.assertEqual((check.returncode, check.stdout.strip()), (0, 'lisp tree: ok'))

    def test_check_names_a_reference_that_came_back(self):
        self.run_all()
        (self.root / 'src/late.rs').write_text('const B: &str = "rlisp/b.el";\n')
        git(self.root, 'add', 'src/late.rs')
        check = tool(self.root, 'check')
        self.assertEqual((check.returncode, check.stdout.strip()), (1, 'src/late.rs: names an old tree'))
        (self.root / 'rlisp').mkdir()
        self.assertIn('rlisp/ still exists', tool(self.root, 'check').stdout)

    def test_check_names_a_link_into_an_old_tree(self):
        self.run_all()
        (self.root / 'link.el').symlink_to('rlisp/b.el')
        git(self.root, 'add', 'link.el')
        self.assertIn('link.el: a link into an old tree', tool(self.root, 'check').stdout)

    def test_move_refuses_an_unresolved_merge(self):
        git(self.root, 'checkout', '-q', '-b', 'lane')
        (self.root / 'rlisp/b.el').write_text('(lane)\n')
        git(self.root, 'commit', '-qam', 'lane edits')
        git(self.root, 'checkout', '-q', 'trunk')
        (self.root / 'rlisp/b.el').write_text('(trunk)\n')
        git(self.root, 'commit', '-qam', 'trunk edits')
        subprocess.run(['git', '-C', str(self.root), 'merge', '-q', 'lane'], capture_output=True)
        moved = tool(self.root, 'move')
        self.assertNotEqual(moved.returncode, 0)
        self.assertIn('unmerged paths', moved.stderr)

    def test_a_second_run_changes_nothing(self):
        self.run_all()
        git(self.root, 'add', '-A')
        git(self.root, 'commit', '-qm', 'moved')
        self.run_all()
        self.assertEqual(git(self.root, 'status', '--porcelain'), '')

    def test_a_branch_catches_up_after_merging_the_move(self):
        git(self.root, 'checkout', '-q', '-b', 'lane')
        (self.root / 'rlisp/new.el').write_text('(load "rlisp/b")\n')
        git(self.root, 'add', 'rlisp/new.el')
        git(self.root, 'commit', '-qm', 'lane adds a file')
        git(self.root, 'checkout', '-q', 'trunk')
        self.run_all()
        git(self.root, 'add', '-A')
        git(self.root, 'commit', '-qm', 'moved')
        git(self.root, 'checkout', '-q', 'lane')
        git(self.root, '-c', 'merge.directoryRenames=false', 'merge', '-q', '--no-edit', 'trunk')
        self.assertTrue((self.root / 'rlisp/new.el').is_file())
        check = self.run_all()
        self.assertEqual(self.read('lisp/romacs/new.el'), '(load "lisp/romacs/b")\n')
        self.assertFalse((self.root / 'rlisp').exists())
        self.assertEqual(check.returncode, 0)


if __name__ == '__main__':
    unittest.main()
