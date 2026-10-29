"""world_diff.py's comparison of two world digests."""
import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / 'tools'))
import world_diff  # noqa: E402


class Compare(unittest.TestCase):
    def test_lists_differing_missing_and_unused_allowed_roots(self):
        a = {('value', 'x'): '1', ('value', 'clock'): '2', ('buffer', 'b'): '3', ('value', 'same'): '4'}
        b = {('value', 'x'): '9', ('value', 'clock'): '8', ('face', 'f'): '5', ('value', 'same'): '4'}
        allow = {('value', 'clock'): 'the clock', ('value', 'same'): 'never differs'}
        bad, ok, unused = world_diff.compare(a, b, allow)
        self.assertEqual(bad, [('buffer', 'b'), ('face', 'f'), ('value', 'x')])
        self.assertEqual(ok, [('value', 'clock')])
        self.assertEqual(unused, [('value', 'same')])

    def test_equal_digests_pass(self):
        a = {('value', 'x'): '1'}
        self.assertEqual(world_diff.compare(a, dict(a), {}), ([], [], []))


if __name__ == '__main__':
    unittest.main()
