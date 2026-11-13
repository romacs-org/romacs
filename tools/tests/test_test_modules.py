#!/usr/bin/env python3
"""tools/lib/test_modules.py: the module list is the directory's files less
the standalone ones, a stale list is rewritten, and `equal` passes only when
`all` holds exactly the tests of its files."""
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import test_modules  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def suite(name, tests, kind='test', package='romacs-vm'):
    return {'package-name': package, 'binary-name': name, 'kind': kind,
            'testcases': {test: {'ignored': test.endswith('_ignored')} for test in tests}}


def inventory(*suites):
    return {f'{s["package-name"]}::{s["binary-name"]}': s for s in suites}


class TestModulesTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        self.tests = self.root / 'crates/romacs-vm/tests'
        (self.tests / 'all').mkdir(parents=True)
        (self.tests / 'support').mkdir()
        for name in ('b', 'a', 'alone', 'support/helper'):
            (self.tests / f'{name}.rs').write_text('')
        (self.tests / 'all/standalone.txt').write_text('# why\nalone: it has an allocator of its own\n')

    def test_the_checked_in_list_is_current(self):
        for name, directory in test_modules.PACKAGES.items():
            text = test_modules.modules_text(test_modules.package(ROOT, name)[0])
            self.assertEqual((ROOT / directory / 'tests/all/modules.rs').read_text(), text)

    def test_modules_are_the_files_less_the_standalone_ones(self):
        self.assertEqual(test_modules.package(self.root, 'romacs-vm'), (['a', 'b'], {'alone': 'it has an allocator of its own'}))
        self.assertEqual(test_modules.targets(self.root, 'romacs-vm'), ['--lib', '--bins', '--test', 'all', '--test', 'alone'])

    def test_a_stale_list_is_rewritten_once(self):
        self.assertEqual(test_modules.write(self.root), ['crates/romacs-vm/tests/all/modules.rs'])
        self.assertEqual(test_modules.write(self.root), [])
        (self.tests / 'c.rs').write_text('')
        self.assertEqual(test_modules.write(self.root), ['crates/romacs-vm/tests/all/modules.rs'])
        self.assertIn('#[path = "../c.rs"]\nmod c;\n', (self.tests / 'all/modules.rs').read_text())

    def test_a_standalone_entry_needs_a_file_and_a_reason(self):
        (self.tests / 'all/standalone.txt').write_text('gone: it was here\n')
        with self.assertRaisesRegex(ValueError, 'names no test file: gone'):
            test_modules.package(self.root, 'romacs-vm')
        for text in ('alone\n', 'alone: \n', 'alone: once\nalone: twice\n'):
            with self.assertRaisesRegex(ValueError, 'is not one `FILE: reason`'):
                test_modules.standalone(text)

    def equal(self, *suites):
        return test_modules.equal(inventory(*suites), 'romacs-vm', ['a', 'b'], {'alone': 'why'})

    def test_equal_passes_when_all_holds_each_files_tests(self):
        apart = (suite('a', ['one', 'deep::two_ignored']), suite('b', ['one']), suite('alone', ['one']),
                 suite('romacs_vm', ['unit'], kind='lib'), suite('a', ['other'], package='romacs-vm-gui'))
        whole = suite('all', ['a::one', 'a::deep::two_ignored', 'b::one', 'its_own_test'])
        self.assertEqual(self.equal(*apart, whole), (3, []))

    def test_equal_names_a_test_on_one_side_only(self):
        apart = (suite('a', ['one']), suite('b', ['one', 'two']), suite('alone', ['one']))
        count, found = self.equal(*apart, suite('all', ['a::one', 'b::one', 'b::three']))
        self.assertEqual(count, 3)
        self.assertEqual(found, ['romacs-vm: b::two is in its own target, not so in `all`',
                                 'romacs-vm: b::three is in `all`, not so in its own target'])

    def test_equal_holds_ignored_tests_to_the_same_state(self):
        apart = (suite('a', ['one_ignored']), suite('b', []), suite('alone', []))
        whole = suite('all', ['a::one_ignored'])
        whole['testcases']['a::one_ignored']['ignored'] = False
        self.assertEqual(len(self.equal(*apart, whole)[1]), 2)

    def test_equal_refuses_an_inventory_that_lacks_a_target_or_any_test(self):
        self.assertIn('no test target listed for all, alone', self.equal(suite('a', ['one']), suite('b', []))[1][0])
        empty = (suite('a', []), suite('b', []), suite('alone', ['one']), suite('all', []))
        self.assertIn('list no test', self.equal(*empty)[1][0])


if __name__ == '__main__':
    unittest.main()
