#!/usr/bin/env python3
"""tools/lib/impact.py: a change selects the tests and harnesses it can reach,
and everything when it cannot be mapped."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/lib'))
import impact  # noqa: E402


def workspace(root):
    """A workspace like the repository's, small: romacs-vm, romacs-vm-gui on it,
    romacs-display-runtime alone; romacs-vm's tests, one folded through a
    support file, romacs-vm-gui's harness and a fixture a test names."""
    files = {
        'Cargo.toml': '[workspace]\nmembers = ["crates/*"]\nresolver = "2"\n',
        'crates/romacs-vm/Cargo.toml': '[package]\nname = "romacs-vm"\nversion = "0.0.0"\nedition = "2021"\n',
        'crates/romacs-vm/src/lib.rs': '',
        'crates/romacs-vm/src/gc.rs': '',
        'crates/romacs-vm/tests/all/standalone.txt': '',
        'crates/romacs-vm/tests/quit_async.rs': '',
        'crates/romacs-vm/tests/status_bar.rs': '#[path = "support/gui_probe.rs"]\nmod gui_probe;\n',
        'crates/romacs-vm/tests/support/gui_probe.rs': '#[path = "gnu_lisp.rs"]\nmod gnu_lisp;\n',
        'crates/romacs-vm/tests/support/gnu_lisp.rs': 'fn f() { root.join("lisp/session/x.el"); }\n',
        'crates/romacs-vm/tests/plain.rs': '',
        'crates/romacs-vm/tests/support/pinned_gnu.rs': '',
        'crates/romacs-vm/tests/filelock.rs': 'include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/pinned_gnu.rs"));\n',
        'crates/romacs-vm/tests/support/orphan.rs': '',
        'crates/romacs-vm-gui/Cargo.toml': '[package]\nname = "romacs-vm-gui"\nversion = "0.0.0"\nedition = "2021"\n'
                                           '[dependencies]\nromacs-vm = { path = "../romacs-vm" }\n',
        'crates/romacs-vm-gui/src/main.rs': 'fn main() {}\n',
        'crates/romacs-vm-gui/tests/all/standalone.txt': '',
        'crates/romacs-vm-gui/tests/scaling.rs': 'const H: &str = "harness/scaling_live.py";\n',
        'crates/romacs-vm-gui/tests/fixtures/sample.el': '',
        'crates/romacs-vm-gui/tests/samples.rs': 'const F: &str = "tests/fixtures/sample.el";\n',
        'crates/romacs-vm-gui/harness/live_oracle.py': '',
        'crates/romacs-vm-gui/harness/scaling_live.py': 'import live_oracle\n',
        'crates/romacs-vm-gui/harness/world_live.py': 'from live_oracle import stop\n',
        'crates/romacs-vm-gui/harness/agent_live.py': '',
        'crates/romacs-display-runtime/Cargo.toml': '[package]\nname = "romacs-display-runtime"\nversion = "0.0.0"\nedition = "2021"\n',
        'crates/romacs-display-runtime/src/lib.rs': '',
    }
    for name, text in files.items():
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        (root / name).write_text(text)
    return impact.metadata_of(root)


class Select(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = Path(tempfile.mkdtemp())
        cls.metadata = workspace(cls.root)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.root)

    def select(self, *paths):
        return impact.select(paths, self.root, self.metadata)

    def test_a_crates_source_reaches_its_tests_and_its_dependents(self):
        found = self.select('crates/romacs-vm/src/gc.rs')
        self.assertEqual(set(found['packages']), {'romacs-vm', 'romacs-vm-gui'})
        self.assertIn('package(romacs-vm-gui)', found['filter'])
        self.assertNotIn('romacs-display-runtime', found['filter'].replace('time_discipline', ''))

    def test_a_crate_nothing_depends_on_reaches_only_itself(self):
        found = self.select('crates/romacs-display-runtime/src/lib.rs')
        self.assertEqual(set(found['packages']), {'romacs-display-runtime'})
        self.assertIn(impact.CRATE_RULES['romacs-display-runtime'], found['filter'])
        self.assertEqual(found['harnesses'], {})

    def test_a_change_the_editor_reaches_calls_for_every_standard_harness(self):
        found = self.select('crates/romacs-vm/src/gc.rs')
        self.assertEqual(set(found['harnesses']), set(impact.HARNESSES))

    def test_a_test_file_reaches_its_tests_alone(self):
        found = self.select('crates/romacs-vm/tests/quit_async.rs')
        self.assertEqual((found['packages'], set(found['files'])), ({}, {'romacs-vm::quit_async'}))
        self.assertIn(impact.test_file('romacs-vm::quit_async'), found['filter'])

    def test_a_test_file_is_named_in_its_packages_folded_targets_only(self):
        self.assertEqual(impact.test_file('romacs-vm-gui::samples'),
                         'binary_id(romacs-vm-gui::samples) | ((binary_id(romacs-vm-gui::all)) & test(/^samples::/))')
        self.assertIn('binary_id(romacs-vm::gnu)', impact.test_file('romacs-vm::plain'))
        self.assertEqual(impact.test_file('romacs-display-runtime::x'), 'binary_id(romacs-display-runtime::x)')

    def test_a_support_file_reaches_the_tests_that_include_it_through_another(self):
        found = self.select('crates/romacs-vm/tests/support/gnu_lisp.rs')
        self.assertEqual(set(found['files']), {'romacs-vm::status_bar'})

    def test_a_support_file_included_by_include_reaches_its_includers(self):
        self.assertEqual(set(self.select('crates/romacs-vm/tests/support/pinned_gnu.rs')['files']), {'romacs-vm::filelock'})

    def test_a_support_file_no_test_names_reaches_its_whole_package(self):
        self.assertIn('romacs-vm', self.select('crates/romacs-vm/tests/support/orphan.rs')['packages'])

    def test_runs_give_one_run_a_package_with_its_own_terms(self):
        runs = impact.runs(self.select('crates/romacs-vm-gui/src/main.rs', 'crates/romacs-vm/tests/quit_async.rs'), self.root)
        found = {package: (flags, terms) for package, flags, terms in runs}
        self.assertEqual(found['romacs-vm-gui'], (['--lib', '--bins', '--test', 'all'], 'package(romacs-vm-gui)'))
        self.assertIn(impact.test_file('romacs-vm::quit_async'), found['romacs-vm'][1])
        self.assertNotIn('romacs-vm-gui', found['romacs-vm'][1])
        self.assertIsNone(impact.runs(self.select('Cargo.lock'), self.root))

    def test_a_package_without_folded_targets_runs_its_test_targets(self):
        # --lib refuses a package that has no library (romacs-gnu-tests); --tests takes what it has.
        runs = {package: flags for package, flags, _ in impact.runs(self.select('crates/romacs-display-runtime/src/lib.rs'), self.root)}
        self.assertEqual(runs['romacs-display-runtime'], ['--tests'])

    def test_runs_can_leave_rules_out(self):
        slow = impact.test_file('romacs-vm::compile_boundaries')
        # A package the change does not reach runs its rules by name: a rule left is not named.
        quick = {package: terms for package, _, terms in
                 impact.runs(self.select('doc/x.org'), self.root, leave=('romacs-vm::compile_boundaries',))}
        self.assertNotIn(slow, quick['romacs-vm'])
        self.assertIn(impact.test_file('romacs-vm::reentry_sites'), quick['romacs-vm'])
        # A package it reaches runs whole, less the rule left.
        whole = {package: terms for package, _, terms in
                 impact.runs(self.select('crates/romacs-vm/src/gc.rs'), self.root, leave=('romacs-vm::compile_boundaries',))}
        self.assertEqual(whole['romacs-vm'], f'(package(romacs-vm)) & not ({slow})')

    def test_a_fixture_reaches_the_tests_that_name_it(self):
        self.assertEqual(set(self.select('crates/romacs-vm-gui/tests/fixtures/sample.el')['files']),
                         {'romacs-vm-gui::samples'})

    def test_a_harness_module_reaches_the_harnesses_that_import_it_and_the_tests_that_run_them(self):
        found = self.select('crates/romacs-vm-gui/harness/live_oracle.py')
        self.assertEqual(set(found['harnesses']), {'scaling', 'world'})
        self.assertEqual(set(self.select('crates/romacs-vm-gui/harness/scaling_live.py')['files']),
                         {'romacs-vm-gui::scaling'})

    def test_runtime_lisp_reaches_its_readers_the_editor_and_the_lisp_tier(self):
        found = self.select('lisp/session/x.el')
        self.assertEqual(set(found['packages']), {'romacs-vm', 'romacs-vm-gui'})
        self.assertEqual(found['lisp'], ['lisp/session/x.el'])
        self.assertEqual(set(found['harnesses']), set(impact.HARNESSES))

    def test_a_rule_adds_its_harness(self):
        self.assertIn('agent', self.select('crates/romacs-vm-gui/harness/agent_live.py')['harnesses'])

    def test_documentation_reaches_only_the_structural_rules(self):
        found = self.select('doc/x.org', 'AGENTS.md', 'tools/tests/test_x.py')
        self.assertEqual((found['packages'], found['files'], found['harnesses']), ({}, {}, {}))
        self.assertEqual(found['filter'], ' | '.join(impact.test_file(b) for b in impact.STRUCTURAL))

    def test_what_cannot_be_mapped_runs_everything(self):
        for path in ('Cargo.lock', 'tools/gate', 'tools/lib/impact.py', 'rust-toolchain.toml'):
            with self.subTest(path):
                found = self.select('crates/romacs-vm/tests/quit_async.rs', path)
                self.assertEqual((found['everything'], found['filter'], found['reasons']), (True, None, [path]))

    def test_the_command_prints_the_same_answer(self):
        found = subprocess.run([sys.executable, '-B', str(ROOT / 'tools/lib/impact.py'), 'select', '--root',
                                str(self.root), 'crates/romacs-vm/tests/quit_async.rs'],
                               capture_output=True, text=True, check=True).stdout
        self.assertIn('"romacs-vm::quit_async"', found)


if __name__ == '__main__':
    unittest.main()
