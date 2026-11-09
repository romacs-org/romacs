#!/usr/bin/env python3
"""tools/lib/unsafe_census.py: what counts as a site and as documented, the
floors ratchet in both directions, and the unsafe_op_in_unsafe_fn ledger."""
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import unsafe_census  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def sites(src):
    return [(kind, ok) for kind, ok, _ in unsafe_census.file_census(src)[0]]


class SitesTest(unittest.TestCase):
    def test_comments_and_literals_are_not_code(self):
        src = '''
// unsafe { in a comment }
/* unsafe { /* nested */ unsafe { } */
fn f<'a>(x: &'a str) -> char {
    let _ = "unsafe { \\" }";
    let _ = r#"unsafe { "# ;
    let _ = b"unsafe {";
    let _ = '"';
    let _ = '\\'';
    'x'
}
'''
        self.assertEqual(sites(src), [])

    def test_a_quote_in_a_char_literal_opens_no_string(self):
        self.assertEqual(sites("let _ = '\"'; unsafe { f() }\nlet _ = b'\"'; unsafe { f() }"),
                         [('block', False), ('block', False)])

    def test_a_raw_identifier_is_no_site(self):
        self.assertEqual(sites('fn r#unsafe() {}'), [])

    def test_kinds(self):
        src = '''
unsafe fn a() {}
pub unsafe extern "C" fn b() {}
unsafe impl Send for T {}
unsafe trait U {}
unsafe extern "C" { fn c(); }
#[unsafe(no_mangle)]
fn d() { unsafe { a() } }
type P = unsafe extern "C" fn(i32);
struct S { f: unsafe fn() }
'''
        self.assertEqual([k for k, _ in sites(src)], ['fn', 'fn', 'impl', 'trait', 'extern', 'attr', 'block'])

    def test_macro_metavariable_names_a_function(self):
        self.assertEqual(sites('macro_rules! m { ($n:ident) => { unsafe fn $n() {} } }'), [('fn', False)])

    def test_safety_comment_directly_above(self):
        self.assertEqual(sites('fn f() {\n    // SAFETY: p is valid.\n    unsafe { g() }\n}'), [('block', True)])
        self.assertEqual(sites('// Safety: lower case counts\nunsafe impl Send for T {}'), [('impl', True)])
        self.assertEqual(sites('// SAFETY: two lines,\n// still one run.\nunsafe { g() }'), [('block', True)])

    def test_attributes_do_not_break_the_run(self):
        self.assertEqual(sites('// SAFETY: ok\n#[allow(unused)]\nunsafe impl Send for T {}'), [('impl', True)])

    def test_blank_lines_and_code_end_the_run(self):
        self.assertEqual(sites('// SAFETY: far\n\nunsafe { g() }'), [('block', False)])
        self.assertEqual(sites('// SAFETY: above the statement\nlet x =\n    unsafe { g() };'), [('block', False)])

    def test_same_line_comment_counts_only_before_the_keyword(self):
        self.assertEqual(sites('/* SAFETY: ok */ unsafe { g() }'), [('block', True)])
        self.assertEqual(sites('unsafe { g() } // SAFETY: too late'), [('block', False)])

    def test_safety_heading_documents_items_not_blocks(self):
        self.assertEqual(sites('/// # Safety\n/// p must be valid.\npub unsafe fn f() {}'), [('fn', True)])
        self.assertEqual(sites('/// # Safety\nunsafe { g() }'), [('block', False)])

    def test_lint_allows_are_counted_in_code_only(self):
        _, allows = unsafe_census.file_census(
            '#![allow(dead_code, unsafe_op_in_unsafe_fn)]\n// #[allow(unsafe_op_in_unsafe_fn)]\n'
            '#![allow(rust_2024_compatibility)]\n')
        self.assertEqual(allows, 2)


def counts(**crates):
    return {name: Counter(sites=s, undocumented=u) for name, (s, u) in crates.items()}


class RatchetTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        subprocess.run(['git', 'init', '-q'], cwd=self.root, check=True)
        (self.root / 'Cargo.toml').write_text('[workspace]\n[workspace.lints.rust]\nunsafe_op_in_unsafe_fn = "deny"\n')
        self.crate('a', '[lints]\nworkspace = true\n')

    def crate(self, name, lints):
        (self.root / name / 'src').mkdir(parents=True, exist_ok=True)
        (self.root / name / 'Cargo.toml').write_text(f'[package]\nname = "{name}"\n{lints}')

    def problems(self, now, crates, pending=None):
        floors = {'rule': '', 'crates': crates, unsafe_census.PENDING: pending or {}}
        return unsafe_census.problems(self.root, floors, now)

    def test_at_the_floor_passes(self):
        self.assertEqual(self.problems(counts(a=(3, 1)), {'a': {'sites': 3, 'undocumented': 1}}), [])

    def test_a_new_site_fails(self):
        found = self.problems(counts(a=(4, 1)), {'a': {'sites': 3, 'undocumented': 1}})
        self.assertEqual(found, ['a: 4 unsafe sites, above its floor 3'])

    def test_a_new_undocumented_site_fails(self):
        found = self.problems(counts(a=(4, 2)), {'a': {'sites': 4, 'undocumented': 1}})
        self.assertEqual(found, ['a: 2 unsafe sites without a SAFETY: comment, above its floor 1'])

    def test_an_unrecorded_crate_has_floors_of_zero(self):
        self.assertIn('b: 1 unsafe sites, above its floor 0', self.problems(counts(b=(1, 0)), {}))

    def test_a_slack_floor_fails_until_lowered(self):
        floors = {'a': {'sites': 5, 'undocumented': 2}, 'gone': {'sites': 1, 'undocumented': 1}}
        now = counts(a=(3, 2))
        self.assertTrue(any('below its floor 5' in p for p in self.problems(now, floors)))
        lowered = unsafe_census.lowered({'rule': '', 'crates': floors}, now)['crates']
        self.assertEqual(lowered, {'a': {'sites': 3, 'undocumented': 2}})
        self.assertEqual(self.problems(now, lowered), [])

    def test_lower_never_raises(self):
        floors = {'rule': '', 'crates': {'a': {'sites': 3, 'undocumented': 1}}}
        self.assertEqual(unsafe_census.lowered(floors, counts(a=(9, 9)))['crates'], floors['crates'])

    def test_dump_round_trips(self):
        floors = {'rule': 'r', 'crates': {'a': {'sites': 3, 'undocumented': 1}}, unsafe_census.PENDING: {'b': 'why'}}
        self.assertEqual(json.loads(unsafe_census.dump(floors)), floors)

    def test_a_crate_that_does_not_deny_the_lint_must_be_pending(self):
        self.crate('b', '')
        self.assertEqual(self.problems({}, {}), [
            'b does not deny unsafe_op_in_unsafe_fn; deny it, or list it in '
            'unsafe_op_in_unsafe_fn_pending with a reason'])
        self.assertEqual(self.problems({}, {}, {'b': 'reason'}), [])
        self.assertIn('without a reason', self.problems({}, {}, {'b': ' '})[0])

    def test_forbidding_unsafe_code_counts_as_denying(self):
        self.crate('b', '[lints.rust]\nunsafe_code = "forbid"\n')
        self.assertEqual(self.problems({}, {}), [])

    def test_a_source_allow_undoes_the_manifest(self):
        (self.root / 'a/src/lib.rs').write_text('#![allow(unsafe_op_in_unsafe_fn)]\n')
        now = unsafe_census.census(self.root)
        self.assertIn('a does not deny', self.problems(now, {})[0])
        self.assertEqual(self.problems(now, {}, {'a': 'reason'}), [])

    def test_a_pending_crate_that_denies_fails(self):
        self.assertIn('a denies unsafe_op_in_unsafe_fn; drop it', self.problems({}, {}, {'a': 'reason'})[0])

    def test_a_pending_name_that_is_no_package_fails(self):
        self.assertIn('names gone, which is no package', self.problems({}, {}, {'gone': 'reason'})[0])

    def test_the_checked_in_tree_matches_its_floors(self):
        self.assertEqual(unsafe_census.problems(ROOT), [])


if __name__ == '__main__':
    unittest.main()
