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
        self.assertEqual(sorted(found), ['crates/a/src/kept.rs k'])

    def test_code_lines_leave_out_blank_and_comment_lines(self):
        src = 'fn a() {\n    // why\n\n    let s = "}{";\n    /* a\n       b */\n    x(\'{\');\n}\n'
        self.assertEqual(functions(src), {'a': (8, 2, False)})

    def test_raw_strings_lifetimes_and_generics_hold_no_braces(self):
        src = ('fn a<\'b, T: Fn() -> [u8; 2]>(x: &\'b str) -> Result<(), ()> where T: Copy {\n'
               '    let r = r#"{"#;\n    Ok(())\n}\n')
        self.assertEqual(functions(src), {'a': (4, 2, False)})

    def test_a_declaration_without_a_body_is_not_a_function(self):
        src = 'trait T {\n    fn a(&self);\n    fn b(&self) {\n    }\n}\n'
        self.assertEqual(list(functions(src)), ['b'])

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
