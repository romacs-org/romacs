#!/usr/bin/env python3
"""tools/lib/host_allow.py: the allow-list merges entry by entry, and its
entries become HOST comments at their sites."""
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import host_allow  # noqa: E402

HEADER = '# A comment.\n'
A = 'a.rs f std::fs 00000001 reads a\n'
B = 'a.rs g std::fs 00000002 reads b\n'


class MergeTest(unittest.TestCase):
    def test_neighbouring_entries_change_apart(self):
        ours = HEADER + 'a.rs f std::fs 0000000a reads a anew\n' + B
        theirs = HEADER + A + 'a.rs g std::fs 0000000b reads b anew\n'
        self.assertEqual(host_allow.merged(HEADER + A + B, ours, theirs),
                         (HEADER + 'a.rs f std::fs 0000000a reads a anew\na.rs g std::fs 0000000b reads b anew\n', []))

    def test_both_sides_add_in_one_gap_and_one_removes(self):
        ours = HEADER + A + 'a.rs f2 std::fs 00000003 reads c\n' + B
        theirs = HEADER + 'a.rs f1 std::fs 00000004 reads d\n' + B
        self.assertEqual(host_allow.merged(HEADER + A + B, ours, theirs),
                         (HEADER + 'a.rs f1 std::fs 00000004 reads d\na.rs f2 std::fs 00000003 reads c\n' + B, []))

    def test_one_entry_changed_two_ways_is_a_conflict_that_keeps_the_rest(self):
        one, other = A.replace('reads a', 'one'), A.replace('reads a', 'other')
        self.assertEqual(host_allow.merged(A, one, other + B), (one + other + B, ['a.rs f std::fs 00000001']))
        self.assertEqual(host_allow.merged(A, '', other + B), (other + B, ['a.rs f std::fs 00000001']))
        self.assertEqual(host_allow.merged(A, one, one), (one, []))

    def test_the_driver_writes_a_conflicted_merge_and_says_so(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            paths = [Path(tmp) / name for name in ('base', 'ours', 'theirs')]
            for path, text in zip(paths, (A, A.replace('reads a', 'one'), A.replace('reads a', 'other') + B)):
                path.write_text(text)
            self.assertEqual(host_allow.merge_files(*paths), 1)
            self.assertEqual(paths[1].read_text(), A.replace('reads a', 'one') + A.replace('reads a', 'other') + B)

    def test_a_key_listed_twice_is_a_conflict(self):
        with self.assertRaisesRegex(ValueError, 'listed twice'):
            host_allow.merged(A, A + A.replace('reads a', 'again'), A)


SOURCE = """\
fn first() {
    let a = std::fs::read("a");
}
fn second() {
    // SAFETY: a valid descriptor.
    let b = unsafe { std::fs::OpenOptions::new() };
    let a = std::fs::read("a");
}
"""


def line_mark(text):
    """The lint's mark of the line of SOURCE holding TEXT."""
    line = next(n for n, l in enumerate(SOURCE.splitlines(), 1) if text in l)
    return host_allow.marks(SOURCE)[line]


class ConvertTest(unittest.TestCase):
    def convert(self, listed, source=SOURCE):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / 'src.rs').write_text(source)
            failed = host_allow.convert(Path(tmp), host_allow.entries(listed))
            return (Path(tmp) / 'src.rs').read_text(), failed

    def test_marks_are_the_lints(self):
        # Two entries of the list the lint wrote, for crates/romacs-vm/src/bin/romacs-vm-image.rs.
        src = "fn run() {\n    let source = std::fs::read(&args[2]).map_err(|e| e.to_string())?;\n}\n"
        self.assertEqual(host_allow.marks(src)[2], '571f88ba06ce7bf3')
        self.assertEqual(host_allow.marks(src.replace('[2]', '[1]'))[2], 'd8db18833735ec7e')

    def test_each_entry_is_a_comment_above_its_line_and_a_rerun_adds_none(self):
        a, b = line_mark('read("a")'), line_mark('OpenOptions')
        listed = (f'src.rs first std::fs {a} reads a\n'
                  f'src.rs second std::fs {a} reads a again\n'
                  f'src.rs second OpenOptions {b} opens b\n'
                  f'src.rs second std::fs {b} opens b\n')
        out, failed = self.convert(listed)
        self.assertEqual(failed, [])
        self.assertEqual(out, """\
fn first() {
    // HOST(std::fs): reads a
    let a = std::fs::read("a");
}
fn second() {
    // SAFETY: a valid descriptor.
    // HOST(OpenOptions, std::fs): opens b
    let b = unsafe { std::fs::OpenOptions::new() };
    // HOST(std::fs): reads a again
    let a = std::fs::read("a");
}
""")
        self.assertEqual(self.convert(listed, out), (out, []))

    def test_a_long_reason_wraps(self):
        reason = ' '.join(['word'] * 40)
        out, _ = self.convert(f'src.rs first std::fs {line_mark(chr(34) + "a")} {reason}\n')
        comments = [l for l in out.splitlines() if l.lstrip().startswith('//') and 'SAFETY' not in l]
        self.assertEqual(comments[0][:30], '    // HOST(std::fs): word wor')
        self.assertTrue(all(len(l) <= host_allow.WIDTH for l in comments) and len(comments) == 3, comments)

    def test_an_entry_without_one_line_is_reported(self):
        listed = 'src.rs first std::fs 0123456789abcdef gone\n'
        out, failed = self.convert(listed)
        self.assertEqual((out, [(e[2], why) for e, why in failed]), (SOURCE, [('std::fs', 'no line, or no one line, has its mark')]))


if __name__ == '__main__':
    unittest.main()
