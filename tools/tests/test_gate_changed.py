"""What the small gate selects, from a real repository's history: what the
change since its base reaches (tools/lib/impact.py)."""
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/lib'))
sys.path.insert(0, str(ROOT / 'tools/tests'))
import impact  # noqa: E402
import test_impact  # noqa: E402
GATE = runpy.run_path(str(ROOT / 'tools/gate'), run_name='gate')
changed_filter = GATE['changed_filter']
ONLY_STRUCTURAL = ' | '.join(impact.test_file(binary) for binary in impact.STRUCTURAL)


class ChangedFilter(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.repo = Path(temp.name)
        self.git('init', '-q', '-b', 'beta')
        test_impact.workspace(self.repo)
        # The copy of the selector the gate loads is the work tree's own.
        (self.repo / 'tools/lib').mkdir(parents=True)
        for name in ('impact.py', 'test_modules.py'):
            (self.repo / 'tools/lib' / name).write_text((ROOT / 'tools/lib' / name).read_text())
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

    def test_a_source_change_selects_its_package_its_dependents_and_the_structural_rules(self):
        self.write('crates/romacs-vm/src/gc.rs')
        selected = changed_filter(self.repo)
        self.assertIn('package(romacs-vm)', selected)
        self.assertIn('package(romacs-vm-gui)', selected)
        self.assertTrue(selected.endswith(ONLY_STRUCTURAL))

    def test_a_test_file_selects_its_binary_and_the_structural_rules(self):
        self.write('crates/romacs-vm/tests/quit_async.rs')
        self.assertEqual(changed_filter(self.repo), impact.test_file('romacs-vm::quit_async') + ' | ' + ONLY_STRUCTURAL)

    def test_docs_and_test_commands_select_only_the_structural_rules(self):
        self.write('doc/x.org', 'AGENTS.md', 'tools/tests/test_x.py')
        self.assertEqual(changed_filter(self.repo), ONLY_STRUCTURAL)

    def test_a_base_leaves_out_what_came_before_it(self):
        self.git('checkout', '-q', '-b', 'trunk')
        self.write('Cargo.lock')
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
        names = ['crates/romacs-vm/src/gc.rs', 'crates/romacs-vm/tests/quit_async.rs', 'doc/x.org']
        self.write(*names)
        self.assertEqual(GATE['filter_for'](names, self.repo), changed_filter(self.repo))
        self.assertIsNone(GATE['filter_for']([*names, 'Cargo.lock'], self.repo))

    def test_what_cannot_be_mapped_runs_everything(self):
        self.write('Cargo.lock')
        self.assertIsNone(changed_filter(self.repo))

    def test_the_crate_rules_and_structural_names_stay_readable_here(self):
        # tools/dev-check reads them from the gate it loads as a module.
        import importlib.machinery
        import importlib.util
        loader = importlib.machinery.SourceFileLoader('romacs_gate', str(ROOT / 'tools/gate'))
        gate = importlib.util.module_from_spec(importlib.util.spec_from_loader(loader.name, loader))
        loader.exec_module(gate)
        self.assertEqual((gate.STRUCTURAL, gate.SLOW_RULES), (impact.STRUCTURAL, impact.SLOW_RULES))
        self.assertEqual(gate.crate_rules({'crates/romacs-display-runtime/src/x.rs'}), impact.CRATE_RULES)


if __name__ == '__main__':
    unittest.main()
