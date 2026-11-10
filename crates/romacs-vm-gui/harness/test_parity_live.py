"""Focused harness tests; no Cargo build or ambient compositor required."""
import unittest

import layout_journey_live
import parity_live


class KeyTests(unittest.TestCase):
    def test_a_character_is_typed_under_its_modifier(self):
        self.assertEqual(parity_live.C('n'), ['-M', 'ctrl', 'n', '-m', 'ctrl'])
        self.assertEqual(parity_live.M('>'), ['-M', 'alt', '>', '-m', 'alt'])

    def test_a_named_key_is_pressed_not_spelled(self):
        # Typed, `space' would be C-s C-p C-a C-c C-e.
        self.assertEqual(parity_live.C('space'), ['-M', 'ctrl', '-k', 'space', '-m', 'ctrl'])
        self.assertEqual(parity_live.M('Return'), ['-M', 'alt', '-k', 'Return', '-m', 'alt'])

    def test_no_journey_key_spells_a_name(self):
        for group in layout_journey_live.DOWN + layout_journey_live.KEYS:
            if '-M' in group:
                key = group[2:-2]
                self.assertTrue(len(key[-1]) == 1 or key[0] == '-k', group)


if __name__ == '__main__':
    unittest.main()
