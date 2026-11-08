#!/usr/bin/env python3
"""The upstream fonts Romacs ships as they are (assets/fonts/sources.json).

Each entry of sources.json is one upstream release: its version, the
release archive's URL and SHA-256 (as the release's SHA-256.txt gives it),
the SHA-256 of every file taken from it, and its licence: the file, the
URL it came from and its SHA-256. `check` hashes the committed
files against that record, so a font cannot change silently, and refuses a
font in assets/fonts that no entry and no build (RomacsIcons.ttf,
tools/icon_font.py) accounts for, and a README that does not give each
file's SHA-256.

usage: bundled_fonts.py check [--root ROOT]
"""
import argparse
import hashlib
import json
import sys
from pathlib import Path

FONTS = Path('assets/fonts')
SOURCES = FONTS / 'sources.json'
# Fonts Romacs builds itself, checked by their own tool.
BUILT = {'RomacsIcons.ttf'}
# What crates/romacs-vm-gui/src/fonts.rs registers.
FONT_SUFFIXES = {'.ttf', '.otf', '.ttc'}


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def problems(root):
    """Why ROOT's bundled fonts do not match sources.json, one line each."""
    fonts = Path(root) / FONTS
    sources = json.loads((Path(root) / SOURCES).read_text())
    readme = (fonts / 'README.md').read_text()
    found = []
    recorded = set()
    for name, release in sorted(sources.items()):
        for key in ('version', 'url', 'sha256', 'files', 'licence'):
            if key not in release:
                found.append(f'{SOURCES}: {name} has no {key}')
        licence = release.get('licence', {})
        if not (fonts / licence.get('file', '')).is_file() or sha256(fonts / licence['file']) != licence.get('sha256'):
            found.append(f'{name}\'s licence {FONTS / licence.get("file", "?")} is missing or not the one {SOURCES} records')
        for file, digest in sorted(release.get('files', {}).items()):
            recorded.add(file)
            path = fonts / file
            if not path.is_file():
                found.append(f'{FONTS / file} is missing ({name} {release.get("version")})')
            elif sha256(path) != digest:
                found.append(f'{FONTS / file} is not the file {SOURCES} records for {name} {release.get("version")}')
            if digest not in readme:
                found.append(f'{FONTS / "README.md"} does not give {file}\'s SHA-256')
    for path in sorted(fonts.iterdir()):
        if path.suffix in FONT_SUFFIXES and path.name not in recorded | BUILT:
            found.append(f'{FONTS / path.name} is in no entry of {SOURCES}')
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=['check'])
    parser.add_argument('--root', default='.')
    args = parser.parse_args()
    found = problems(args.root)
    for problem in found:
        print(f'bundled-fonts: {problem}', file=sys.stderr)
    return 1 if found else 0


if __name__ == '__main__':
    sys.exit(main())
