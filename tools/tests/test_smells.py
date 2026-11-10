#!/usr/bin/env python3
"""tools/lib/smells.py: what counts as each smell, the lint census's dedup and
attribution, and the floors ratchet in both directions."""
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
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


class TreeTest(unittest.TestCase):
    def test_the_floors_file_is_in_dump_form(self):
        root = Path(__file__).resolve().parents[2]
        text = (root / smells.FLOORS).read_text()
        self.assertEqual(smells.dump(json.loads(text)), text)


if __name__ == '__main__':
    unittest.main()
