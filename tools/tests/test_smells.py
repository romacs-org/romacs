#!/usr/bin/env python3
"""tools/lib/smells.py: what counts as each smell, the lint census's dedup and
attribution, and the floors ratchet in both directions."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import smells  # noqa: E402


def smell(src, long_file_lines=3):
    return smells.file_smells(src, long_file_lines)


class FileSmellsTest(unittest.TestCase):
    def test_allows_count_each_dead_or_unused_lint_name(self):
        src = '''
#![allow(dead_code)]
#[allow(unused, unused_imports, clippy::unused_self)]
#[expect(dead_code, reason = "allow(dead_code) in a reason")]
#[cfg_attr(test, allow(unused_mut))]
#[allow(clippy::too_many_lines)]
'''
        self.assertEqual(smell(src, 100)[0]['allows'], 5)

    def test_comments_and_strings_hold_no_allows_and_code_holds_no_todos(self):
        src = '// #[allow(dead_code)] TODO\nlet s = "#[allow(unused)] FIXME";\nlet TODO = 1;\n'
        found = smell(src, 100)[0]
        self.assertEqual((found['allows'], found['todos']), (0, 1))

    def test_todos_are_whole_words(self):
        self.assertEqual(smell('// TODO, FIXME: XXX HACK; TODOs XXXL hack\n', 100)[0]['todos'], 4)

    def test_a_long_file_is_over_the_limit(self):
        self.assertEqual(smell('a\nb\nc\n')[0]['long_files'], 0)
        self.assertEqual(smell('a\nb\nc\nd')[0]['long_files'], 1)

    def test_a_primitive_is_declared_in_code_only(self):
        src = 'defsubr! { name = "a", gnu = "data.c:1" }\ndefsubr!{name="b"}\n$crate::prim::defsubr!(@emit $name)'
        self.assertEqual(smell(src)[1], 2)
        self.assertEqual(smell('// defsubr! { name = "a" }\nmacro_rules! defsubr { () => {} }')[1], 0)


def span(file='src/lib.rs', line=1, called_from=None):
    return {'is_primary': True, 'file_name': file, 'line_start': line, 'column_start': 1,
            'expansion': called_from and {'span': called_from}}


def message(manifest, code, at=None, level='warning', text='m'):
    return json.dumps({'reason': 'compiler-message', 'manifest_path': manifest,
                       'message': {'level': level, 'code': code and {'code': code}, 'message': text,
                                   'spans': [at or span()]}})


FINISHED = json.dumps({'reason': 'build-finished', 'success': True})


class LintCensusTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.root)
        for crate in ('a', 'b'):
            (Path(self.root) / crate).mkdir()
            (Path(self.root) / crate / 'Cargo.toml').write_text(f'[package]\nname = "pkg-{crate}"\n')
        subprocess.run(['git', 'init', '-q', self.root], check=True)
        self.a, self.b = str(Path(self.root) / 'a/Cargo.toml'), str(Path(self.root) / 'b/Cargo.toml')

    def census(self, *lines):
        return smells.lint_census(self.root, [*lines, FINISHED])

    def test_one_count_per_diagnostic_credited_to_its_package(self):
        self.assertEqual(self.census(message(self.a, 'clippy::doc_markdown'),
                                     message(self.a, 'clippy::doc_markdown'),     # a second target
                                     message(self.a, 'clippy::doc_markdown', span(line=2)),
                                     message(self.b, 'dead_code'),
                                     message(self.b, None),                       # "N warnings emitted"
                                     message(self.b, 'dead_code', level='note'),
                                     'Compiling pkg-b'),
                         {'pkg-a': Counter({'clippy::doc_markdown': 2}), 'pkg-b': Counter({'dead_code': 1})})

    def test_one_file_under_several_names_is_one_place(self):
        absolute = str(Path(self.root) / 'a/src/lib.rs')
        self.assertEqual(self.census(message(self.a, 'dead_code', span('a/src/lib.rs')),
                                     message(self.a, 'dead_code', span('a/tests/../src/lib.rs')),
                                     message(self.a, 'dead_code', span(absolute))),
                         {'pkg-a': Counter({'dead_code': 1})})

    def test_a_warning_inside_a_shared_macro_counts_per_call(self):
        core = '/toolchain/library/core/src/macros/mod.rs'
        calls = [message(self.a, 'clippy::float_cmp', span(core, 40, span('a/src/lib.rs', line)))
                 for line in (1, 2, 2)]
        self.assertEqual(self.census(*calls), {'pkg-a': Counter({'clippy::float_cmp': 2})})

    def test_a_failed_or_cut_off_build_counts_nothing(self):
        warning = message(self.a, 'dead_code')
        failed = json.dumps({'reason': 'build-finished', 'success': False})
        for lines in ([warning], [warning, failed], [FINISHED, warning]):
            with self.assertRaises(ValueError):
                smells.lint_census(self.root, lines)


def floors(**layout):
    return {'rule': 'r', 'long_file_lines': 1500,
            'crates': {'a': {'allows': 2, 'todos': 1, 'long_files': 1}},
            'lints': {'clippy::doc_markdown': 3},
            'layout': {'root': ['crates', 'tools'], 'members_outside_crates': ['tests/x'],
                       'legacy_primitives': 5, 'primitives_outside_prims': 2, 'generated': 3, **layout}}


def now(allows=2, todos=1, long_files=1, lint=3, root=('crates', 'tools'), outside=('tests/x',), legacy=5,
        strays=2):
    return {'crates': {'a': Counter(allows=allows, todos=todos, long_files=long_files)},
            'lints': Counter({'clippy::doc_markdown': lint}) if lint else Counter(),
            'layout': {'root': list(root), 'members_outside_crates': list(outside), 'legacy_primitives': legacy,
                       'primitives_outside_prims': strays, 'generated': 3},
            'strays': {}, 'long': [], 'lints_by_crate': {}}


class RatchetTest(unittest.TestCase):
    def test_at_the_floors(self):
        self.assertEqual(smells.problems(floors(), now()), [])

    def test_a_rise_fails(self):
        found = smells.problems(floors(), now(allows=3, lint=4, legacy=6))
        self.assertEqual(len(found), 3)
        self.assertTrue(all('above its floor' in p or 'above its ceiling' in p for p in found))

    def test_a_lint_below_its_ceiling_passes_until_lowered(self):
        self.assertEqual(smells.problems(floors(), now(lint=1)), [])
        self.assertEqual(smells.lowered(floors(), now(lint=1))['lints'], {'clippy::doc_markdown': 1})

    def test_a_new_crate_or_lint_has_floors_of_zero(self):
        state = now()
        state['crates']['b'] = Counter(todos=1)
        state['lints']['clippy::new_lint'] = 1
        self.assertEqual(smells.problems(floors(), state),
                         ['b todos: 1, above its floor 0', 'lint clippy::new_lint: 1, above its ceiling 0'])

    def test_a_slack_floor_fails_until_lowered(self):
        state = now(allows=1, lint=0, legacy=4, root=('crates',), outside=())
        found = smells.problems(floors(), state)
        self.assertEqual(len(found), 4)
        self.assertTrue(all('lower' in p for p in found))
        low = smells.lowered(floors(), state)
        self.assertEqual(smells.problems(low, state), [])
        self.assertEqual(low['crates'], {'a': {'allows': 1, 'todos': 1, 'long_files': 1}})
        self.assertEqual(low['lints'], {})
        self.assertEqual(low['layout'], {'root': ['crates'], 'members_outside_crates': [], 'legacy_primitives': 4,
                                         'primitives_outside_prims': 2, 'generated': 3})

    def test_lower_never_raises_or_adds(self):
        state = now(allows=9, lint=9, legacy=9, root=('crates', 'tools', 'new'), outside=('tests/x', 'tests/y'))
        self.assertEqual(smells.lowered(floors(), state), floors())

    def test_a_new_top_level_entry_or_outside_member_fails(self):
        found = smells.problems(floors(), now(root=('crates', 'tools', 'logs'), outside=('tests/x', 'tests/y')))
        self.assertEqual(found, ['layout root: logs is new; adding one is a deliberate edit of SMELL-FLOORS.json',
                                 'layout members_outside_crates: tests/y is new; adding one is a deliberate '
                                 'edit of SMELL-FLOORS.json'])

    def test_a_primitive_declared_outside_the_tree_fails(self):
        self.assertEqual(smells.problems(floors(), now(strays=3)),
                         ['layout primitives_outside_prims: 3, above its floor 2'])

    def test_a_crate_at_zero_leaves_the_file(self):
        self.assertEqual(smells.lowered(floors(), now(allows=0, todos=0, long_files=0))['crates'], {})

    def test_the_dump_is_the_floors(self):
        self.assertEqual(json.loads(smells.dump(floors())), floors())


def counts(lint=3, allows=2, legacy=5, root=('crates', 'tools'), generated=3):
    """A tree's counts, or a floors file, in the floors file's shape."""
    return {'crates': {'a': {'allows': allows, 'todos': 1, 'long_files': 1}},
            'lints': {'clippy::doc_markdown': lint} if lint else {},
            'layout': {'root': list(root), 'members_outside_crates': ['tests/x'], 'legacy_primitives': legacy,
                       'primitives_outside_prims': 2, 'generated': generated}}


def rise(what, have, had, over=1):
    return (f"{what}: {have}, above the {had} of its merge-base"
            f"; remove {over}, or set its floor in SMELL-FLOORS.json {over} above the merge-base's")


class MergeBaseTest(unittest.TestCase):
    """rises(floors, the merge-base's floors, the merge-base's counts, the tree's counts)."""

    def test_floors_lowered_since_the_fork_do_not_fail_an_unchanged_branch(self):
        # The branch forked at 5 and still has 5; the trunk has since dropped to 3
        # and lowered the floor. Held to the trunk's floors the branch fails for
        # what it never touched; held to its merge-base it passes.
        trunk_floors = floors()
        branch = now(lint=5)
        self.assertEqual(smells.problems(trunk_floors, branch),
                         ['lint clippy::doc_markdown: 5, above its ceiling 3'])
        forked = counts(lint=5)
        self.assertEqual(smells.rises(forked, forked, forked, counts(lint=5)), [])

    def test_a_rise_fails_under_a_slack_floor(self):
        slack = counts(lint=9, allows=9, legacy=9)
        self.assertEqual(smells.rises(slack, slack, counts(), counts(lint=4, allows=3, legacy=6)),
                         [rise('a allows', 3, 2),
                          rise('layout legacy_primitives', 6, 5),
                          rise('lint clippy::doc_markdown', 4, 3)])

    def test_a_drop_needs_no_record(self):
        self.assertEqual(smells.rises(counts(), counts(), counts(), counts(lint=0, allows=0, legacy=1)), [])

    def test_a_raised_floor_allows_a_rise_of_as_much(self):
        was, raised = counts(lint=9), counts(lint=11)
        self.assertEqual(smells.rises(raised, was, counts(), counts(lint=5)), [])
        self.assertEqual(smells.rises(raised, was, counts(), counts(lint=6)),
                         ["lint clippy::doc_markdown: 6, above the 3 of its merge-base and the 2 its floor was raised by"
                          "; remove 1, or set its floor in SMELL-FLOORS.json 3 above the merge-base's"])

    def test_the_same_raise_on_both_sides_merges_to_one_and_the_failure_says_what_to_set(self):
        # The branch and the trunk each added one and raised the floor 9 -> 10.
        # Git merges the two identical edits to 10, not 11, so the branch's own
        # raise is gone from the file; the rise fails, and names the floor to set.
        merged = counts(lint=10)
        self.assertEqual(smells.rises(merged, merged, counts(lint=6), counts(lint=7)),
                         ["lint clippy::doc_markdown: 7, above the 6 of its merge-base"
                          "; remove 1, or set its floor in SMELL-FLOORS.json 1 above the merge-base's"])
        self.assertEqual(smells.rises(counts(lint=11), merged, counts(lint=6), counts(lint=7)), [])

    def test_a_lowered_floor_allows_nothing_and_forbids_nothing(self):
        self.assertEqual(smells.rises(counts(lint=1), counts(lint=9), counts(), counts()), [])

    def test_a_new_lint_or_crate_is_a_rise_from_zero(self):
        tree = counts()
        tree['lints']['clippy::new_lint'] = 1
        tree['crates']['b'] = {'allows': 0, 'todos': 1, 'long_files': 0}
        self.assertEqual(smells.rises(counts(), counts(), counts(), tree),
                         [rise('b todos', 1, 0),
                          rise('lint clippy::new_lint', 1, 0)])
        listed = counts()
        listed['crates']['b'] = {'allows': 0, 'todos': 1, 'long_files': 0}
        listed['lints']['clippy::new_lint'] = 1
        self.assertEqual(smells.rises(listed, counts(), counts(), tree), [])

    def test_a_new_top_level_entry_is_listed_or_fails(self):
        tree = counts(root=('crates', 'tools', 'logs'))
        self.assertEqual(smells.rises(counts(), counts(), counts(), tree),
                         ['layout root: logs is new; adding one is a deliberate edit of SMELL-FLOORS.json'])
        listed = counts(root=('crates', 'tools', 'logs'))
        self.assertEqual(smells.rises(listed, counts(), counts(), tree), [])
        # A listing the merge-base already had is not this change's: `logs` was
        # deleted, its listing not yet lowered away, and it may not come back.
        self.assertEqual(len(smells.rises(listed, listed, counts(), tree)), 1)
        self.assertEqual(smells.rises(counts(), counts(), counts(), counts(root=('crates',))), [])

    def test_generated_is_held_to_its_floor(self):
        self.assertEqual(smells.rises(counts(), counts(), counts(), counts(generated=4)),
                         ['layout generated: 4, above its floor 3'])
        self.assertEqual(smells.rises(counts(), counts(), counts(), counts(generated=2)), [])

    def test_a_merge_base_from_before_the_floors_file(self):
        empty = {'crates': {}, 'lints': {}, 'layout': {}}
        self.assertEqual(smells.rises(counts(), empty, empty, counts()), [])


class BaseCountsTest(unittest.TestCase):
    """The merge-base is found by git, measured in the kept checkout, and kept.
    A tree's count here is the number in its file `n`."""

    def setUp(self):
        self.repo = tempfile.mkdtemp()
        self.cache = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.repo)
        self.addCleanup(shutil.rmtree, self.cache)
        self.measured = []
        saved = smells.measure, smells.counting, smells.cache_dir
        self.addCleanup(lambda: setattr(smells, 'measure', saved[0]) or setattr(smells, 'counting', saved[1])
                        or setattr(smells, 'cache_dir', saved[2]))
        smells.measure = self.measure
        smells.counting = lambda long_file_lines: self.how
        smells.cache_dir = lambda: Path(self.cache)
        self.how = 'counted one way'
        self.git('init', '-q', '-b', 'trunk')
        self.commit(5)
        self.git('checkout', '-q', '-b', 'branch')

    def git(self, *args):
        return subprocess.run(['git', '-c', 'user.name=t', '-c', 'user.email=t@example.org', '-c',
                               'commit.gpgsign=false', *args], cwd=self.repo, check=True,
                              stdout=subprocess.PIPE, text=True).stdout.strip()

    def commit(self, n, floor=9, name='n'):
        (Path(self.repo) / name).write_text(str(n))
        (Path(self.repo) / smells.FLOORS).write_text(smells.dump({'rule': 'r', 'long_file_lines': 1500,
                                                                  **counts(lint=floor)}))
        self.git('add', '-A')
        self.git('commit', '-q', '-m', f'{name} {n}')

    def measure(self, root, floors, clippy_json=None):
        self.measured.append(str(root))
        return now(lint=int((Path(root) / 'n').read_text()))

    def check(self, base='trunk'):
        floors = json.loads((Path(self.repo) / smells.FLOORS).read_text())
        tree = smells.as_floors(None, 1500, self.measure(self.repo, floors))
        self.measured.clear()
        return smells.rises(floors, *smells.base_side(self.repo, base, floors, tree), tree)

    def test_the_branch_is_held_to_its_fork_not_to_the_trunk_since(self):
        self.commit(5, name='other')                      # the branch: unchanged in the lint
        self.git('checkout', '-q', 'trunk')
        self.commit(3, floor=3)                           # the trunk drops, and lowers the floor
        self.git('checkout', '-q', 'branch')
        self.assertEqual(self.check(), [])
        self.assertEqual(self.measured, [str(Path(self.cache) / 'base')])
        self.commit(6)
        self.assertEqual(self.check(), [rise('lint clippy::doc_markdown', 6, 5)])
        self.assertEqual(self.measured, [], 'the merge-base was kept')

    def test_merging_the_trunk_moves_the_merge_base(self):
        self.commit(5, name='other')
        self.git('checkout', '-q', 'trunk')
        self.commit(3, floor=3)
        self.git('checkout', '-q', 'branch')
        self.git('merge', '-q', '--no-edit', 'trunk')
        self.assertEqual(self.check(), [])
        self.commit(4, floor=3)
        self.assertEqual(self.check(), [rise('lint clippy::doc_markdown', 4, 3)])
        self.commit(4, floor=4)                           # the deliberate raise
        self.assertEqual(self.check(), [])

    def test_a_clean_tree_keeps_its_own_counts_for_later_branches(self):
        self.assertEqual(self.check(), [])                # the branch is the trunk's tip
        self.assertEqual(self.measured, [], 'a tree that is its own merge-base is measured once')
        (Path(self.repo) / 'n').write_text('7')           # unstaged: not the tree git has
        self.assertEqual(self.check(), [rise('lint clippy::doc_markdown', 7, 5, 2)])
        self.assertEqual(self.measured, [])

    def test_where_branch_and_trunk_merged_each_other_the_trunks_own_line_is_the_base(self):
        self.commit(5, name='other')                      # the branch's first work, which the trunk will merge
        early = self.git('rev-parse', 'HEAD')
        self.git('checkout', '-q', 'trunk')
        self.commit(3, floor=3)                           # the trunk drops to 3
        self.git('checkout', '-q', 'branch')
        self.git('merge', '-q', '--no-edit', 'trunk')     # the branch takes the trunk in: 3
        self.git('checkout', '-q', 'trunk')
        self.git('merge', '-q', '--no-ff', '--no-edit', early)    # and the trunk merges the early work
        self.git('checkout', '-q', 'branch')
        self.assertEqual(len(self.git('merge-base', '--all', 'HEAD', 'trunk').split()), 2)
        self.assertEqual(self.check(), [])                # held to the trunk at 3, not to its own early 5
        self.commit(4, floor=3)
        self.assertEqual(len(self.check()), 1)

    def test_supplied_diagnostics_are_not_kept_as_a_trees_counts(self):
        floors = json.loads((Path(self.repo) / smells.FLOORS).read_text())
        smells.base_side(self.repo, 'trunk', floors)      # as main calls it under --clippy-json
        self.assertEqual(self.measured, [str(Path(self.cache) / 'base')])

    def test_counts_are_kept_per_way_of_counting(self):
        self.commit(5, name='other')
        self.check()
        self.how = 'counted another way'
        self.check()
        self.assertEqual(self.measured, [str(Path(self.cache) / 'base')])

    def test_check_holds_to_the_merge_base_unless_absolute(self):
        self.commit(5, name='other')
        self.git('checkout', '-q', 'trunk')
        self.commit(3, floor=3)
        self.git('checkout', '-q', 'branch')
        self.git('merge', '-q', '--no-edit', 'trunk')     # counts 3, under a floor of 3
        self.commit(2, floor=3)                           # a drop, with no record
        for flags, status in ((['--base', 'trunk'], 0), (['--absolute'], 0), (['--base', 'no-such-branch'], 1)):
            with mock.patch.object(sys, 'argv', ['smells.py', 'check', '--root', self.repo, *flags]):
                self.assertEqual(smells.main(), status, flags)
        self.commit(4, floor=3)                           # a rise of one
        for flags in (['--base', 'trunk'], ['--absolute']):
            with mock.patch.object(sys, 'argv', ['smells.py', 'check', '--root', self.repo, *flags]):
                self.assertEqual(smells.main(), 1, flags)

    def test_no_such_trunk_is_an_error(self):
        with self.assertRaises(ValueError):
            self.check(base='no-such-branch')


class CargoSettingsTest(unittest.TestCase):
    def settings(self, home, **env):
        with mock.patch.dict(os.environ, {'CARGO_HOME': home, **env}):
            return smells.cargo_settings()

    def test_what_changes_warnings_changes_the_settings_and_where_a_build_goes_does_not(self):
        home = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, home)
        plain = self.settings(home)
        self.assertEqual(plain, self.settings(home, CARGO_TARGET_DIR='/elsewhere', CARGO_BUILD_JOBS='2'))
        self.assertNotEqual(plain, self.settings(home, CARGO_PROFILE_DEV_DEBUG_ASSERTIONS='false'))
        self.assertNotEqual(plain, self.settings(home, RUSTFLAGS='--cfg x'))
        (Path(home) / 'config.toml').write_text('[profile.dev]\ndebug-assertions = false\n')
        self.assertNotEqual(plain, self.settings(home))


class TreeTest(unittest.TestCase):
    def test_the_floors_file_is_in_dump_form(self):
        root = Path(__file__).resolve().parents[2]
        text = (root / smells.FLOORS).read_text()
        self.assertEqual(smells.dump(json.loads(text)), text)


if __name__ == '__main__':
    unittest.main()
