#!/usr/bin/env python3
"""ARCHITECTURE.md names every workspace crate, so each is two hops from the root."""
from pathlib import Path
import re
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ArchitectureMap(unittest.TestCase):
    def test_every_workspace_crate_is_named(self):
        members = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['members']
        text = (ROOT / 'ARCHITECTURE.md').read_text()
        missing = [m for m in members if not re.search(rf'`(?:crates/)?{re.escape(Path(m).name)}[`/]', text)]
        self.assertEqual(missing, [], 'name each in ARCHITECTURE.md "Where things are"')


if __name__ == '__main__':
    unittest.main()
