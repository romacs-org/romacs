#!/usr/bin/env python3
"""tools/lib/unsafe_census.py: what counts as a site, as documented and as
declared, and the rule that every package denies unsafe_op_in_unsafe_fn."""
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import unsafe_census  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def sites(src):
    return [(kind, ok) for kind, ok, _, _ in unsafe_census.file_census(src)[0]]


def declared(src):
    return [declared for _, _, declared, _ in unsafe_census.file_census(src)[0]]


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

    def test_unsafe_with_its_reason_beside_the_safety_comment_is_declared(self):
        src = ('// SAFETY: p is valid.\n// UNSAFE(no crate maps a file descriptor)\nunsafe { g() }\n'
               '// SAFETY: p is valid. UNSAFE()\nunsafe { g() }\n'
               '// UNSAFE(far)\n\n// SAFETY: ok\nunsafe { g() }\n')
        self.assertEqual(declared(src), [True, False, False])
        self.assertEqual(sites(src), [('block', True)] * 3)

    def test_lint_allows_are_counted_in_code_only(self):
        _, allows = unsafe_census.file_census(
            '#![allow(dead_code, unsafe_op_in_unsafe_fn)]\n// #[allow(unsafe_op_in_unsafe_fn)]\n'
            '#![allow(rust_2024_compatibility)]\n')
        self.assertEqual(allows, 2)


class LintRuleTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.root)
        subprocess.run(['git', 'init', '-q'], cwd=self.root, check=True)
        (self.root / 'Cargo.toml').write_text('[workspace]\n[workspace.lints.rust]\nunsafe_op_in_unsafe_fn = "deny"\n')
        self.crate('a', '[lints]\nworkspace = true\n')

    def crate(self, name, lints):
        (self.root / name / 'src').mkdir(parents=True, exist_ok=True)
        (self.root / name / 'Cargo.toml').write_text(f'[package]\nname = "{name}"\n{lints}')

    def problems(self):
        return unsafe_census.lint_problems(self.root, unsafe_census.census(self.root))

    def test_every_package_denies_the_lint(self):
        self.assertEqual(self.problems(), [])
        self.crate('b', '')
        self.assertEqual(self.problems(), ['b does not deny unsafe_op_in_unsafe_fn; deny it'])

    def test_forbidding_unsafe_code_counts_as_denying(self):
        self.crate('b', '[lints.rust]\nunsafe_code = "forbid"\n')
        self.assertEqual(self.problems(), [])

    def test_a_source_allow_undoes_the_manifest(self):
        (self.root / 'a/src/lib.rs').write_text('#![allow(unsafe_op_in_unsafe_fn)]\n')
        self.assertEqual(self.problems(), ['a does not deny unsafe_op_in_unsafe_fn; deny it'])

    def test_the_tree_keeps_the_rule(self):
        self.assertEqual(unsafe_census.lint_problems(ROOT, unsafe_census.census(ROOT)), [])


if __name__ == '__main__':
    unittest.main()
