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

    def test_an_allowed_root_gone_from_one_side_differs(self):
        a = {('value', 'clock'): '1', ('value', 'x'): '2'}
        b = {('value', 'x'): '2'}
        allow = {('value', 'clock'): 'the clock'}
        bad, ok, unused = world_diff.compare(a, b, allow)
        self.assertEqual((bad, ok), ([('value', 'clock')], []))
        bad, ok, unused = world_diff.compare(b, a, allow)
        self.assertEqual((bad, ok), ([('value', 'clock')], []))


class Parse(unittest.TestCase):
    def test_a_complete_digest_parses(self):
        lines = ['value\tx\tab', 'buffer\ta\tb\tcd', 'end\tlines\t2']
        self.assertEqual(world_diff.parse(lines), {('value', 'x'): 'ab', ('buffer', 'a\tb'): 'cd'})

    def test_an_empty_or_cut_digest_is_refused(self):
        for lines in ([], ['end\tlines\t0'], ['value\tx\tab'], ['value\tx\tab', 'value\ty\tcd', 'end\tlines\t1'],
                      ['value\tx\tab', 'value\tx\tab', 'end\tlines\t2']):
            with self.assertRaises(world_diff.Incomplete, msg=lines):
                world_diff.parse(lines)

    def test_two_empty_inputs_do_not_pass(self):
        import tempfile, os
        with tempfile.TemporaryDirectory() as d:
            a, b = os.path.join(d, 'a'), os.path.join(d, 'b')
            for path in (a, b):
                open(path, 'w').close()
            self.assertEqual(world_diff.main([a, b]), 2)


if __name__ == '__main__':
    unittest.main()
