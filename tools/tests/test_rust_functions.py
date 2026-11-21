#!/usr/bin/env python3
"""tools/lib/rust_functions.py: which Rust is in scope, what a function and a
code line are, what declares a long function, and the floors."""
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import rust_functions as rf  # noqa: E402

ROOT = Path(__file__).resolve().parents[2]


def functions(src):
    return {name: (lines, code, declared) for name, _, lines, code, declared in rf.file_census(src)[0]}


class ScopeTest(unittest.TestCase):
    def test_own_rust_is_crates_less_vendor_tests_and_benches(self):
        paths = ('crates/a/src/lib.rs', 'crates/a/src/x/tests.rs', 'crates/a/tests/t.rs', 'crates/a/tests/all/m.rs',
                 'crates/a/benches/b.rs', 'crates/vendor/v/src/lib.rs', 'tools/x.rs', 'crates/a/build.rs')
        self.assertEqual([p for p in paths if rf.own(p)],
                         ['crates/a/src/lib.rs', 'crates/a/src/x/tests.rs', 'crates/a/build.rs'])


class FunctionsTest(unittest.TestCase):
    def test_a_line_of_a_string_that_spans_lines_is_code(self):
        self.assertEqual(functions('fn a() {\n    let s = "one\n\ntwo";\n}\n'), {'a': (5, 2, False)})

    def test_a_test_module_in_its_own_file_is_left_out(self):
        found, _ = rf.census([('crates/a/src/lib.rs', '#[cfg(test)]\nmod tests;\n#[cfg(test)]\n#[path = "x_tests.rs"]\nmod x;\n'),
                              ('crates/a/src/tests.rs', 'fn t() {\n}\n'), ('crates/a/src/x_tests.rs', 'fn u() {\n}\n'),
                              ('crates/a/src/kept.rs', 'fn k() {\n}\n')])
        self.assertEqual(sorted(found), ['crates/a/src/kept.rs kept::k'])

    def test_code_lines_leave_out_blank_and_comment_lines(self):
        src = 'fn a() {\n    // why\n\n    let s = "}{";\n    /* a\n       b */\n    x(\'{\');\n}\n'
        self.assertEqual(functions(src), {'a': (8, 2, False)})

    def test_raw_strings_lifetimes_and_generics_hold_no_braces(self):
        src = ('fn a<\'b, T: Fn() -> [u8; 2]>(x: &\'b str) -> Result<(), ()> where T: Copy {\n'
               '    let r = r#"{"#;\n    Ok(())\n}\n')
        self.assertEqual(functions(src), {'a': (4, 2, False)})

    def test_a_declaration_without_a_body_is_not_a_function(self):
        src = 'trait T {\n    fn a(&self);\n    fn b(&self) {\n    }\n}\n'
        self.assertEqual(list(functions(src)), ['T::b'])

    def test_a_nested_function_counts_on_its_own_and_in_its_parent(self):
        src = 'fn outer() {\n    fn inner() {\n        x();\n    }\n    inner();\n}\n'
        found = functions(src)
        self.assertEqual((found['outer'][1], found['inner'][1]), (4, 1))

    def test_test_items_are_left_out(self):
        src = ('fn kept() {\n}\n#[cfg(test)]\nmod tests {\n    fn helper() { x().unwrap(); }\n}\n'
               '#[test]\nfn check() {\n    y().unwrap();\n}\n#[cfg(test)]\nuse z;\nfn after() { w.expect("e"); }\n')
        census, unwraps, _ = rf.file_census(src)
        self.assertEqual([f[0] for f in census], ['kept', 'after'])
        self.assertEqual(unwraps, 1)

    def test_a_declaration_is_in_the_comment_lines_above_attributes_stepped_over(self):
        src = '// SMELL(long-function): one table\n#[inline]\npub fn a() {\n}\n// SMELL(long-function):\nfn b() {\n}\n'
        found = functions(src)
        self.assertEqual((found['a'][2], found['b'][2]), (True, False))


class ReviewCasesTest(unittest.TestCase):
    def test_a_cfg_that_requires_test_is_test_only(self):
        src = ('#[cfg(all(test, feature = "x"))]\nfn t() { a.unwrap(); }\n'
               '#[cfg(any(test, feature = "x"))]\nfn k() { b.unwrap(); }\n'
               '#[cfg(not(test))]\nfn n() {\n}\n')
        census, unwraps, _ = rf.file_census(src)
        self.assertEqual(([f[0] for f in census], unwraps), (['k', 'n'], 1))

    def test_a_test_item_ends_at_its_own_end(self):
        census, unwraps, _ = rf.file_census('#[cfg(test)]\nfn t(_: [u8; 1]) {\n    a.unwrap();\n}\nfn k() { b.unwrap(); }\n'
                                            'fn p(#[cfg(test)] x: u8) {\n    c.unwrap();\n}\n')
        self.assertEqual(([f[0] for f in census], unwraps), (['k', 'p'], 2))

    def test_test_modules_read_from_code_with_their_path(self):
        src = ('/* #[cfg(test)] mod support; */\nconst S: &str = "#[cfg(test)] mod other;";\n'
               '#[path = "x_tests.rs"]\n#[cfg(test)]\nmod x;\n')
        code = rf.unsafe_census.lex(src)[0]
        self.assertEqual(rf.test_modules('crates/a/src/lib.rs', src, code), {'crates/a/src/x_tests.rs'})

    def test_a_const_argument_in_the_signature_is_not_the_body(self):
        body = '    x();\n' * 60
        found = functions(f'fn f() -> S<{{ 1 + 2 }}> where T: Q<{{ 3 }}> {{\n{body}}}\n')
        self.assertEqual(found['f'][1], 60)

    def test_a_statement_beside_a_brace_is_a_code_line(self):
        self.assertEqual(functions('fn f() { a();\n    b();\n    c(); }\n')['f'][1], 3)
        self.assertEqual(functions('fn f(\n    x: u8,\n) {}\n')['f'][1], 0)

    def test_a_method_is_named_by_its_impl_or_trait(self):
        src = ('impl<T: Copy> Grid<T> {\n    fn get(&self) {\n    }\n}\nimpl Display for Grid<u8> {\n    fn fmt(&self) {\n    }\n}\n'
               'trait Draw {\n    fn draw(&self) {\n    }\n}\nmod inner {\n    fn get() {\n    }\n}\n')
        self.assertEqual(sorted(functions(src)), ['Draw::draw', 'Grid<T>::get', 'Grid<u8> as Display::fmt', 'inner::get'])

    def test_each_new_long_function_is_a_finding_and_names_the_rust_marker(self):
        body = '    x();\n' * 60
        base = rf.summary(rf.census([('crates/a/src/lib.rs', f'fn old() {{\n{body}}}\n')]))
        tree = rf.summary(rf.census([('crates/a/src/lib.rs', f'fn new() {{\n{body}{body}}}\n')]))
        (new,) = rf.rises(base, tree)
        self.assertTrue(new.startswith('long_rust_functions: crates/a/src/lib.rs:1: new is new at 120 code lines'), new)
        self.assertIn('`// SMELL(long-function): reason`', new)

    def test_the_marker_is_above_attributes_however_many_lines(self):
        src = ('// SMELL(long-function): a table\n#[allow(\n    clippy::x,\n)]\npub(crate) async fn a() {\n}\n')
        self.assertTrue(functions(src)['a'][2])


class RoundTwoTest(unittest.TestCase):
    def test_a_test_field_or_statement_owns_only_itself(self):
        src = ('struct Table {\n    #[cfg(test)]\n    pub(crate) probe: u8,\n    real: u8,\n}\n'
               'impl Default for Table {\n    fn default() -> Self {\n        x.unwrap()\n    }\n}\n'
               'fn mark() {\n    #[cfg(test)]\n    if checking {\n        y.unwrap();\n    }\n    z();\n}\n')
        census, unwraps, _ = rf.file_census(src)
        self.assertEqual(([f[0] for f in census], unwraps), (['Table as Default::default', 'mark'], 1))

    def test_a_function_moved_from_an_inline_module_to_its_file_keeps_its_name(self):
        body = '    x();\n' * 60
        base = rf.summary(rf.census([('crates/a/src/lib.rs', f'mod inner {{\n    fn kept() {{\n{body}    }}\n}}\n')]))
        tree = rf.summary(rf.census([('crates/a/src/inner.rs', f'fn kept() {{\n{body}}}\n')]))
        self.assertEqual(rf.rises(base, tree), [])

    def test_nested_generics_name_the_impl_whole(self):
        src = "impl<'a, T: Into<Vec<u8>>> Scope<'a, T> {\n    fn enter(&self) {\n    }\n}\n"
        self.assertEqual(list(functions(src)), ["Scope<'a, T>::enter"])

    def test_a_token_macro_holds_no_function(self):
        self.assertEqual(list(functions('fn real() {\n    let s = stringify!(fn tokens() { x() });\n}\n')), ['real'])

    def test_a_spaced_attribute_still_shows_its_marker(self):
        self.assertTrue(functions('// SMELL(long-function): x\n# [allow(\n    y,\n)]\nfn a() {\n}\n')['a'][2])

    def test_a_comparison_in_an_array_length_is_no_generic(self):
        self.assertEqual(list(functions('fn f(_: [u8; (1 < 2) as usize]) {\n    x();\n}\n')), ['f'])

    def test_a_test_module_path_is_normalized_and_found_in_an_inline_module(self):
        src = '#[cfg(test)]\n#[path = "../helpers.rs"]\nmod helpers;\nmod outer {\n    #[cfg(test)]\n    mod t;\n}\n'
        code = rf.unsafe_census.lex(src)[0]
        self.assertEqual(rf.test_modules('crates/a/src/x/lib.rs', src, code),
                         {'crates/a/src/helpers.rs', 'crates/a/src/x/outer/t.rs', 'crates/a/src/x/outer/t/mod.rs'})


class RoundThreeTest(unittest.TestCase):
    def test_a_test_shorthand_field_owns_only_itself(self):
        src = ('fn make() -> S {\n    S {\n        #[cfg(test)]\n        available,\n        other: 1,\n    }\n}\n'
               'impl S {\n    fn try_send(&self) {\n        x.unwrap();\n    }\n}\n')
        census, unwraps, _ = rf.file_census(src)
        self.assertEqual(([f[0] for f in census], unwraps), (['make', 'S::try_send'], 1))

    def test_a_span_ends_at_its_enclosing_group(self):
        src = 'fn f() {\n    g(\n        #[cfg(test)]\n        1\n    );\n    y.unwrap();\n}\n'
        self.assertEqual(rf.file_census(src)[1], 1)

    def test_an_explicit_path_in_an_inline_module_resolves_under_it(self):
        src = 'mod outer {\n    #[cfg(test)]\n    #[path = "./helper.rs"]\n    mod t;\n}\n'
        code = rf.unsafe_census.lex(src)[0]
        self.assertEqual(rf.test_modules('crates/a/src/lib.rs', src, code), {'crates/a/src/outer/helper.rs'})
        src = '#[path = "."]\nmod outer {\n    #[cfg(test)]\n    #[path = "helper.rs"]\n    mod t;\n}\n'
        code = rf.unsafe_census.lex(src)[0]
        self.assertEqual(rf.test_modules('crates/a/src/lib.rs', src, code), {'crates/a/src/helper.rs'})


class FloorsTest(unittest.TestCase):
    def test_unwraps_and_undocumented_unsafe_outside_tests(self):
        src = ('fn a() {\n    x.unwrap();\n    y.unwrap_or(1);\n    z.expect("why");\n    // "s.unwrap()"\n'
               '    unsafe { f() };\n    // SAFETY: g is sound here.\n    unsafe { g() };\n}\n'
               '#[cfg(test)]\nmod t {\n    fn b() { unsafe { h() } }\n}\n')
        _, unwraps, undocumented = rf.file_census(src)
        self.assertEqual((unwraps, undocumented), (2, 1))

    def test_a_rise_of_a_floor_is_a_finding(self):
        base = {'long': {}, 'keys': [], 'floors': {'unwraps': 3, 'unsafe_undocumented': 1}}
        tree = {'long': {}, 'keys': [], 'floors': {'unwraps': 4, 'unsafe_undocumented': 0}}
        (finding,) = rf.rises(base, tree)
        self.assertTrue(finding.startswith('long_rust_functions: unwraps: 4 in non-test Rust, above the 3'), finding)

    def test_a_long_function_is_held_as_a_lisp_one_is(self):
        body = '    x();\n' * 60
        base = rf.summary(rf.census([('crates/a/src/lib.rs', f'fn f() {{\n{body}}}\n')]))
        tree = rf.summary(rf.census([('crates/a/src/lib.rs', f'fn f() {{\n{body}    y();\n}}\n')]))
        self.assertEqual(json.loads(json.dumps(base)), base, 'a summary is kept as JSON')
        (grown,) = rf.rises(base, tree)
        self.assertTrue(grown.startswith('long_rust_functions: crates/a/src/lib.rs:1: f grew to 61 code lines from 60'), grown)
        self.assertEqual(rf.rises(base, base), [])


class TreeTest(unittest.TestCase):
    def test_the_tree_reads(self):
        found, floors = rf.tree_census(ROOT)
        self.assertGreater(len(found), 5000)
        self.assertEqual(set(floors), {'unwraps', 'unsafe_undocumented'})


if __name__ == '__main__':
    unittest.main()
