#!/usr/bin/env python3
"""Romacs's icon font as a derived artifact (tools/build-icon-font).

assets/fonts/RomacsIcons.ttf records, in its name table, the SHA-256 of
what it was built from (name ID 256: every assets/icons/*.svg, the
codepoint list and the build script) and of what it holds (name ID 257:
its cmap, glyf, hmtx and loca tables). `check` compares both records with
the sources and the font now, without fontTools, so a changed drawing or
codepoint, a stale font and a damaged one are caught before the gate.

usage: icon_font.py check [--root ROOT]
"""
import argparse
import hashlib
import struct
import sys
from pathlib import Path

FONT = 'assets/fonts/RomacsIcons.ttf'
SOURCES_NAME_ID = 256
PAYLOAD_NAME_ID = 257
PAYLOAD_TABLES = (b'cmap', b'glyf', b'hmtx', b'loca')


def sources_digest(root):
    """The SHA-256 of the font's sources, in a fixed order."""
    root = Path(root)
    digest = hashlib.sha256()
    files = sorted((root / 'assets/icons').glob('*.svg')) + [root / 'assets/icons/codepoints.txt',
                                                              root / 'tools/build-icon-font']
    for path in files:
        digest.update(path.relative_to(root).as_posix().encode() + b'\0' + path.read_bytes() + b'\0')
    return digest.hexdigest()


def tables(data):
    """The font DATA's tables, by tag."""
    count = struct.unpack_from('>H', data, 4)[0]
    found = {}
    for i in range(count):
        tag, _, offset, length = struct.unpack_from('>4sIII', data, 12 + 16 * i)
        found[tag] = data[offset:offset + length]
    return found


def payload_digest(data):
    """The SHA-256 of the font DATA's glyphs: its PAYLOAD_TABLES."""
    found = tables(data)
    digest = hashlib.sha256()
    for tag in PAYLOAD_TABLES:
        digest.update(tag + found.get(tag, b'') + b'\0')
    return digest.hexdigest()


def recorded(data, name_id):
    """The string the font DATA's name table records under NAME_ID, or None."""
    name = tables(data).get(b'name', b'')
    if len(name) < 6:
        return None
    _, count, strings = struct.unpack_from('>HHH', name, 0)
    for r in range(count):
        platform, _, _, found, length, at = struct.unpack_from('>HHHHHH', name, 6 + 12 * r)
        if found == name_id and platform == 3:
            return name[strings + at:strings + at + length].decode('utf-16-be')
    return None


def check_bytes(data, root):
    """Why the font DATA does not match ROOT's sources or itself, or None."""
    if recorded(data, SOURCES_NAME_ID) != sources_digest(root):
        return f'{FONT} is stale: its sources changed; run tools/build-icon-font'
    if recorded(data, PAYLOAD_NAME_ID) != payload_digest(data):
        return f'{FONT} is damaged: its glyphs are not the ones it was built with; run tools/build-icon-font'
    return None


def check(root):
    font = Path(root) / FONT
    if not font.exists():
        return f'{FONT} is missing: run tools/build-icon-font'
    return check_bytes(font.read_bytes(), root)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=['check'])
    parser.add_argument('--root', default='.')
    args = parser.parse_args()
    problem = check(args.root)
    if problem:
        print(f'icon-font: {problem}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
