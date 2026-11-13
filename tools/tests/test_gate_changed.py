"""What the small gate selects, from a real repository's history."""
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
GATE = runpy.run_path(str(ROOT / 'tools/gate'), run_name='gate')
STRUCTURAL = GATE['STRUCTURAL']
changed_filter = GATE['changed_filter']
filter_for = GATE['filter_for']
test_file = GATE['test_file']
ONLY_STRUCTURAL = ' | '.join(test_file(binary) for binary in STRUCTURAL)


class ChangedFilter(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.repo = Path(temp.name)
        self.git('init', '-q', '-b', 'beta')
        self.write('README.md')
        self.git('add', '.')
        self.git('commit', '-q', '-m', 'base')

    def git(self, *args):
        # A throwaway fixture: no identity or signing key of the caller's.
        subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@t', '-c', 'commit.gpgsign=false', *args],
                       cwd=self.repo, check=True)

    def write(self, *names):
        for name in names:
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('x\n')

    def assert_structural(self, selected):
        for binary in STRUCTURAL:
            self.assertIn(f'binary_id({binary})', selected)

    def test_a_module_selects_its_tests_and_the_structural_rules(self):
        self.write('crates/romacs-vm/src/source_cache/prepared.rs')
        selected = changed_filter(self.repo)
        self.assertIn('prepared', selected)
        self.assert_structural(selected)

    def test_the_reentry_census_is_always_selected(self):
        # A new call into Lisp fails this census from any module, whatever its name.
        self.assertIn('romacs-vm::reentry_sites', STRUCTURAL)

    def test_a_test_file_selects_its_binary_and_the_structural_rules(self):
        self.write('crates/romacs-vm/tests/quit_async.rs')
        selected = changed_filter(self.repo)
        self.assertIn('binary_id(romacs-vm::quit_async)', selected)
        self.assert_structural(selected)

    def test_a_test_file_is_selected_as_a_module_of_all_too(self):
        # A run builds a file's own target or its package's `all`, not both.
        self.assertEqual(test_file('romacs-vm::quit_async'),
                         'binary_id(romacs-vm::quit_async) | (binary_id(romacs-vm::all) & test(/^quit_async::/))')

    def test_docs_and_test_commands_select_only_the_structural_rules(self):
        self.write('doc/x.org', 'AGENTS.md', 'Makefile', 'tools/tests/test_x.py')
        selected = changed_filter(self.repo)
        self.assertEqual(selected, ONLY_STRUCTURAL)

    def test_a_base_leaves_out_what_came_before_it(self):
        self.git('checkout', '-q', '-b', 'trunk')
        self.write('lisp/romacs/x.el')
        self.git('add', '.')
        self.git('commit', '-q', '-m', 'unmappable, on trunk')
        self.git('checkout', '-q', '-b', 'lane')
        self.write('crates/romacs-vm/tests/quit_async.rs')
        self.assertIsNone(changed_filter(self.repo))
        self.assertIn('binary_id(romacs-vm::quit_async)', changed_filter(self.repo, 'trunk'))

    def test_a_clone_without_the_local_trunk_uses_origins(self):
        self.git('checkout', '-q', '-b', 'lane')
        self.git('update-ref', 'refs/remotes/origin/beta', 'beta')
        self.git('branch', '-q', '-D', 'beta')
        self.write('crates/romacs-vm/tests/quit_async.rs')
        self.assertIn('binary_id(romacs-vm::quit_async)', changed_filter(self.repo))

    def test_paths_select_as_the_same_change_in_a_repository_does(self):
        names = ['crates/romacs-vm/src/source_cache/prepared.rs', 'crates/romacs-vm/tests/quit_async.rs', 'doc/x.org']
        self.write(*names)
        self.assertEqual(filter_for(names), changed_filter(self.repo))
        self.assertIsNone(filter_for([*names, 'lisp/romacs/x.el']))
        self.assertEqual(filter_for([]), ONLY_STRUCTURAL)

    def test_what_cannot_be_mapped_runs_everything(self):
        for name in ('lisp/romacs/x.el', 'crates/romacs-vm/src/lib.rs'):
            with self.subTest(name):
                self.write(name)
                self.assertIsNone(changed_filter(self.repo))
                (self.repo / name).unlink()


if __name__ == '__main__':
    unittest.main()
