#!/usr/bin/env python3
"""tools/lib/host_allow.py: the allow-list's entries become HOST comments at
their sites."""
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lib'))
import host_allow  # noqa: E402

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

    def test_a_changed_reason_keeps_the_list_and_added_entries_still_land(self):
        a, b = line_mark('read("a")'), line_mark('OpenOptions')
        trunk = f'src.rs first std::fs {a} reads a\n'
        out, _ = self.convert(trunk)
        branch = (f'src.rs first std::fs {a} reads a, now for a reason the branch changed\n'
                  f'src.rs second OpenOptions {b} opens b\n')
        again, failed = self.convert(branch, out)
        self.assertEqual([(e[2], why.split(';')[0]) for e, why in failed],
                         [('std::fs', 'line 3 has a HOST(std::fs) comment with another reason')])
        self.assertIn('// HOST(OpenOptions): opens b', again)
        self.assertEqual(self.convert(trunk, out), (out, []), 'the same reason is no change')
        shorter = f'src.rs first std::fs {a} reads\n'
        self.assertEqual([e[2] for e, _ in self.convert(shorter, out)[1]], ['std::fs'], 'a shortened reason is a change too')


if __name__ == '__main__':
    unittest.main()
