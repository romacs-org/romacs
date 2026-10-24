#!/usr/bin/env python3
"""Index VM2 primitive declaration locations, not the complete runtime surface.

Run without arguments to regenerate PRIMITIVES.json, --check to validate the
source parser, or NAME ... to look up declaration locations. Lisp definitions
and generated runtime registrations are outside this source index's scope.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
VM2 = ROOT / 'crates/romacs-vm/src'


def source_fingerprint(inputs):
    paths = {ROOT / 'tools/primitive_index.py'}
    for name in inputs:
        path = ROOT / name
        paths.update(path.rglob('*.rs') if path.is_dir() else [path])
    digest = hashlib.sha256()
    for path in sorted(paths):
        data = path.read_bytes()
        digest.update(rel(path).encode('utf-8') + b'\0')
        digest.update(str(len(data)).encode('ascii') + b'\0')
        digest.update(data)
    return digest.hexdigest()


def rel(path):
    return str(path.relative_to(ROOT))


def vm2_primitives():
    entries = {}
    source = VM2 / 'lib.rs'
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
    for source in sorted((VM2 / 'prims').glob('*.rs')):
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


def build():
    inputs = ['crates/romacs-vm/src/lib.rs', 'crates/romacs-vm/src/prims',
              'crates/romacs-vm/src/prim.rs', 'tools/primitive_index.py']
    entries = vm2_primitives()
    return {
        'provenance': {
            'generated_by': 'tools/primitive_index.py',
            'source_fingerprint': source_fingerprint(inputs),
            'inputs': inputs,
        },
        'counts_note': 'Source declarations only; absence is not evidence that a runtime primitive is absent.',
        'counts': {'parsed_total': len(entries), 'parsed_vm2_only': len(entries)},
        'primitives': entries,
    }


def write_if_changed(out, index):
    content = (json.dumps(index, indent=1, ensure_ascii=False) + '\n').encode('utf-8')
    if out.exists() and out.read_bytes() == content:
        return 'unchanged'
    out.write_bytes(content)
    return 'written'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('names', nargs='*')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    index = build()
    if args.names:
        for name in args.names:
            print(json.dumps(index['primitives'].get(name, {'name': name, 'status': 'not found in source declarations'}), ensure_ascii=False))
    elif args.check:
        print(f"{index['counts']['parsed_total']} VM2 source declarations parsed")
    else:
        out = ROOT / 'PRIMITIVES.json'
        print(f"{write_if_changed(out, index)} {rel(out)}: {index['counts']['parsed_total']} VM2 source declarations")
    return 0


if __name__ == '__main__':
    sys.exit(main())
