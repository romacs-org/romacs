"""The records tools/treesit-fuzz compares hold every node and cursor getter
Lisp can reach.

Lisp reaches tree-sitter's nodes only through the wrappers in
crates/romacs-vm/src/treesit/native.rs: the boundary test holds every other
file of crates/romacs-vm/src to naming neither `tree_sitter` nor an
`ffi::ts_` call (BOUNDARY_EXEMPT names the two exceptions: Q's query
engine under treesit/query/, which reads grammar tables and query bytes
through ffi, and the query differential that checks it). WRAPPERS maps
each public method of native.rs's `Node` and `Cursor` to the binding
getters its body calls; a test holds the map to the wrappers' methods, and
another holds every getter in it to GETTERS or LOOKUPS in
tools/lib/treesit-fuzz/src/main.rs, which the fuzz records for every node
and looked-up position. tools/treesit-fuzz runs these before it
builds. The census guards against honest omission: a new wrapper, or a
getter left out of the worker. It does not parse Rust, so code written to
hide a call from it would pass."""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SOURCES = ROOT / 'crates/romacs-vm/src'
NATIVE = SOURCES / 'treesit/native.rs'
WORKER = ROOT / 'tools/lib/treesit-fuzz/src/main.rs'
BINDING = ROOT / 'crates/vendor/tree-sitter/binding_rust/lib.rs'
# Files besides native.rs that may name the runtime, and why.
BOUNDARY_EXEMPT = {
    'treesit/query/': "Q's query engine reads grammar tables and query bytes through ffi; the query differential checks it",
    'treesit/query_oracle_tests.rs': "the query differential itself, a test that drives the runtime to compare query engines",
}
# Each public method of native.rs's Node and Cursor, and the binding getters its body calls.
WRAPPERS = {
    'Node.is_null': (), 'Node.same': (), 'Node.identity': (),
    'Node.is_named': ('is_named',), 'Node.is_missing': ('is_missing',), 'Node.is_extra': ('is_extra',),
    'Node.has_error': ('has_error',), 'Node.next_sibling': ('next_sibling',), 'Node.prev_sibling': ('prev_sibling',),
    'Node.next_named_sibling': ('next_named_sibling',), 'Node.prev_named_sibling': ('prev_named_sibling',),
    'Node.kind': ('kind',), 'Node.start': ('start_byte',), 'Node.end': ('end_byte',),
    'Node.child_count': ('child_count', 'named_child_count'), 'Node.child': ('child', 'named_child'),
    'Node.field_name_for_child': ('field_name_for_child',), 'Node.child_by_field_name': ('child_by_field_name',),
    'Node.descendant_for_range': ('descendant_for_byte_range', 'named_descendant_for_byte_range'),
    'Node.sexp': ('to_sexp',),
    'Cursor.new': ('walk',), 'Cursor.node': ('node',), 'Cursor.parent': ('goto_parent',),
    'Cursor.first_child': ('goto_first_child',), 'Cursor.next_sibling': ('goto_next_sibling',),
    'Cursor.previous_sibling': ('goto_previous_sibling',),
    'Cursor.first_child_for_byte': ('goto_first_child_for_byte',),
}


def boundary_breaches():
    """The files of crates/romacs-vm/src, besides native.rs and the exempt, that name the runtime."""
    breaches = []
    for path in sorted(SOURCES.rglob('*.rs')):
        relative = path.relative_to(SOURCES).as_posix()
        if path == NATIVE or any(relative.startswith(prefix) for prefix in BOUNDARY_EXEMPT):
            continue
        if re.search(r'\btree_sitter\b|ffi::ts_', path.read_text()):
            breaches.append(relative)
    return breaches


def wrapper_methods():
    """The public methods of native.rs's Node and Cursor, as `Type.name`."""
    text = NATIVE.read_text()
    methods = set()
    for kind in ('Node', 'Cursor'):
        block = text[text.index(f'\nimpl {kind} {{'):]
        block = block[:block.index('\n}\n')]
        names = set(re.findall(r'pub\(crate\) fn ([a-z_0-9]+)', block))
        for listed in re.findall(r'^\s*[a-z_]+! \{([^}]*)\}', block, re.M):
            names |= {name.strip() for name in listed.split(',') if name.strip()}
        methods |= {f'{kind}.{name}' for name in names}
    return methods


def recorded(text):
    """The names GETTERS and LOOKUPS in the worker give their records."""
    names = set()
    for table in ('const GETTERS', 'const LOOKUPS'):
        block = text[text.index(table):]
        names |= set(re.findall(r'^    \("([a-z_]+)", ', block[:block.index('\n];')], re.M))
    return names


def missing():
    """The binding getters a wrapper calls that the worker does not record."""
    return sorted({g for getters in WRAPPERS.values() for g in getters} - recorded(WORKER.read_text()))


class Census(unittest.TestCase):
    def test_only_native_rs_names_the_runtime(self):
        self.assertEqual(boundary_breaches(), [])

    def test_wrappers_lists_every_wrapper(self):
        self.assertEqual(sorted(wrapper_methods() ^ set(WRAPPERS)), [])

    def test_wrapped_getters_are_the_bindings(self):
        text = BINDING.read_text()
        api = set()
        for header in ("impl<'tree> Node<'tree> {", "impl<'tree> TreeCursor<'tree> {"):
            block = text[text.index(header):]
            api |= set(re.findall(r'pub fn ([a-z_0-9]+)', block[:block.index('\n}\n')]))
        self.assertEqual(sorted({g for getters in WRAPPERS.values() for g in getters} - api), [])

    def test_every_wrapped_getter_is_recorded(self):
        self.assertEqual(missing(), [])

    def test_named_getters_are_recorded(self):
        self.assertTrue({'grammar_name', 'field_name_for_child', 'goto_first_child_for_byte',
                         'descendant_for_point_range'} <= recorded(WORKER.read_text()))

    def test_a_dropped_getter_is_caught(self):
        worker = WORKER.read_text().replace('    ("field_name_for_child", ', '    ("x_field", ')
        self.assertIn('field_name_for_child', {g for v in WRAPPERS.values() for g in v} - recorded(worker))


if __name__ == '__main__':
    unittest.main()
