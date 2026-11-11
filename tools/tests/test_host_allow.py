#!/usr/bin/env python3
"""tools/lib/host_allow.py: the allow-list merges entry by entry."""
import sys
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


if __name__ == '__main__':
    unittest.main()
