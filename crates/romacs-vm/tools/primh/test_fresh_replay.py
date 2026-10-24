#!/usr/bin/env python3
"""Regression tests for explicit semantic pin preservation."""
import unittest
from fresh_replay import preserve_semantic_pins


class PinPreservation(unittest.TestCase):
    def test_preserves_value_condition_and_data_pins(self):
        pins = {'v': 'VALUE', 'c': 'CONDITION', 'd': 'DATA', 'p': 'PRINTER'}
        self.assertEqual(preserve_semantic_pins(pins, {'q': {}}),
                         {'v': 'VALUE', 'c': 'CONDITION', 'd': 'DATA', 'q': 'PRINTER'})
        self.assertEqual(pins['p'], 'PRINTER')

    def test_refuses_semantic_to_printer_reclassification(self):
        with self.assertRaisesRegex(RuntimeError, 'semantic pins changed'):
            preserve_semantic_pins({'v': 'VALUE'}, {'v': {}})


if __name__ == '__main__':
    unittest.main()
