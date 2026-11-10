#!/usr/bin/env python3
"""Where each primitive is declared, read from the source when asked.

Two routes declare primitives: `defsubr!` in crates/romacs-vm/src/prims/, and
lib.rs's own `PRIMITIVES` table. NAME ... prints each one's declaration as
JSON; no argument prints the counts. Nothing is written: the build makes the
same facts for the binary (OUT_DIR/primitives.json). Lisp definitions and
runtime registrations are outside this index.
"""
from __future__ import annotations
import argparse
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
VM2 = ROOT / 'crates/romacs-vm/src'


def vm2_primitives(src=VM2):
    """{name: declaration record} for both routes in the source tree SRC."""
    rel = lambda path: path.relative_to(src.parents[2]).as_posix()  # noqa: E731
    entries = {}
    source = src / 'lib.rs'
    text = source.read_text()
    table = re.search(r'const PRIMITIVES:\s*&\[\(&str,\s*Primitive\)\]\s*=\s*&\[(.*?)\n\];', text, re.S)
    if table is None:
        raise ValueError('VM2 control primitive table not found')
    for match in re.finditer(r'\(\s*"([^"]+)"\s*,\s*Primitive::(\w+)\s*\)', table.group(1)):
        entries[match[1]] = {
            'name': match[1], 'implementation': 'vm2',
            'variant': 'Primitive::' + match[2],
            'registered_at': f'{rel(source)}:{text[:table.start(1) + match.start()].count(chr(10)) + 1}',
        }
    declaration = re.compile(r'^[ \t]*defsubr!\s*\{\s*name\s*=\s*"([^"]+)"', re.M)
    declarations = 0
    for source in sorted((src / 'prims').glob('*.rs')):
        text = source.read_text()
        for match in declaration.finditer(text):
            name = match[1]
            if name in entries:
                raise ValueError('duplicate VM2 primitive: ' + name)
            entries[name] = {
                'name': name, 'implementation': 'vm2',
                'registered_at': f'{rel(source)}:{text[:match.start()].count(chr(10)) + 1}',
            }
            declarations += 1
        if len(re.findall(r'^[ \t]*defsubr!\s*\{', text, re.M)) != len(list(declaration.finditer(text))):
            raise ValueError('unrecognized primitive declaration in ' + rel(source))
    if not entries or not declarations:
        raise ValueError('VM2 primitive source index is empty')
    return dict(sorted(entries.items()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('names', nargs='*')
    args = parser.parse_args()
    entries = vm2_primitives()
    for name in args.names:
        print(json.dumps(entries.get(name, {'name': name, 'status': 'not found in source declarations'}),
                         ensure_ascii=False))
    if not args.names:
        legacy = sum(1 for e in entries.values() if 'variant' in e)
        print(f'{len(entries)} primitives: {len(entries) - legacy} defsubr!, {legacy} in lib.rs\'s table')
    return 0


if __name__ == '__main__':
    sys.exit(main())
