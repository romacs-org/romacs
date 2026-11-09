#!/usr/bin/env python3
"""Payload admission regressions for the ordinary slot runner."""
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


class SlotPayloadTests(unittest.TestCase):
    def setUp(self):
        source = Path(__file__).with_name('neomacs-slot-runner.py')
        spec = importlib.util.spec_from_file_location('slot_runner', source)
        assert spec is not None and spec.loader is not None
        self.runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.runner)
        temporary = tempfile.TemporaryDirectory(dir=os.environ.get('TMPDIR'))
        self.addCleanup(temporary.cleanup)
        self.runner.STATE = Path(temporary.name)
        self.ticket = self.runner.STATE / '00000000000000000001.json'
        self.command = ['payload', '--argument', 'value with spaces']

    def test_delayed_slot_payload_requires_live_ticket(self):
        for state, ticket in [
            ('missing', None),
            ('mismatched', dict(active=True, unit='different-unit')),
            ('inactive', dict(active=False, unit='old-unit')),
            ('unadmitted', dict(unit='old-unit')),
        ]:
            with self.subTest(state=state):
                self.ticket.unlink(missing_ok=True)
                if ticket is not None:
                    self.runner.save(self.ticket, ticket)
                with patch.object(self.runner.os, 'execvp') as execute:
                    self.assertEqual(
                        self.runner.slot_payload('1', 'old-unit', self.command), 125)
                    execute.assert_not_called()

    def test_slot_payload_executes_active_matching_ticket(self):
        self.runner.save(self.ticket, dict(active=True, unit='old-unit'))
        with patch.object(self.runner.os, 'execvp') as execute:
            self.runner.slot_payload('1', 'old-unit', self.command)
            execute.assert_called_once_with('payload', self.command)


if __name__ == '__main__':
    unittest.main()
