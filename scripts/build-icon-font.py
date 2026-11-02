#!/usr/bin/env python3
"""Build assets/fonts/RomacsIcons.ttf from assets/icons/*.svg.

Each SVG is one glyph: a single <path> in a box 500 wide and 1000 tall,
the baseline at y=800 (DIVERGENCES.md VM2-MODE-LINE). assets/icons/
codepoints.txt gives each icon's name its codepoint in the Supplementary
Private Use Area-A (U+FE000-U+FE0FF). The font is 1000 units per em,
every advance 500, ascent 800 and descent 200, and its name table names
the family Romacs Icons under the SIL Open Font License 1.1.

The build is deterministic: no timestamps, glyphs in codepoint order.
The name table records the sources' digest (tools/icon_font.py), which
scripts/check-derived compares without fontTools. With --check it rebuilds
into memory and exits 1 when the checked-in TTF differs. Needs fontTools (pinned through the flake's nixpkgs:
`nix build --inputs-from . nixpkgs#python3.pkgs.fonttools`).

usage: build-icon-font.py [--check] [ROOT]
"""
import io
import re
import sys
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.cu2quPen import Cu2QuPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.svgLib.path import parse_path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'tools'))
import icon_font  # noqa: E402

FAMILY = 'Romacs Icons'
VERSION = '1.000'
COPYRIGHT = 'Copyright 2026 Thanos Apollo <public@thanosapollo.org>, with Reserved Font Name Romacs Icons'
LICENSE = ('This Font Software is licensed under the SIL Open Font License, Version 1.1. '
           'This license is available with a FAQ at: https://openfontlicense.org')
ADVANCE, ASCENT, DESCENT, BASELINE = 500, 800, 200, 800


def codepoints(root):
    found = {}
    for line in (root / 'assets/icons/codepoints.txt').read_text().splitlines():
        if line.strip() and not line.startswith('#'):
            name, point = line.split()
            found[name] = int(point.removeprefix('U+'), 16)
    return found


def glyph(svg):
    d = re.search(r'\sd="([^"]+)"', svg).group(1)
    pen = TTGlyphPen(None)
    # SVG y grows down from the top of the box; font y up from the baseline.
    parse_path(d, TransformPen(Cu2QuPen(pen, max_err=1.0, reverse_direction=True), (1, 0, 0, -1, 0, BASELINE)))
    return pen.glyph()


def build(root):
    points = codepoints(root)
    names = sorted(points, key=points.get)
    missing = [n for n in names if not (root / f'assets/icons/{n}.svg').exists()]
    if missing:
        raise SystemExit(f'no SVG for {missing}')
    order = ['.notdef'] + names
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(order)
    fb.setupCharacterMap({points[n]: n for n in names})
    glyphs = {'.notdef': TTGlyphPen(None).glyph()}
    glyphs.update({n: glyph((root / f'assets/icons/{n}.svg').read_text()) for n in names})
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({n: (ADVANCE, glyphs[n].xMin if hasattr(glyphs[n], 'xMin') else 0) for n in order})
    fb.setupHorizontalHeader(ascent=ASCENT, descent=-DESCENT)
    fb.setupNameTable({'familyName': FAMILY, 'styleName': 'Regular', 'uniqueFontIdentifier': f'{FAMILY} {VERSION}',
                       'fullName': f'{FAMILY} Regular', 'psName': 'RomacsIcons-Regular', 'version': f'Version {VERSION}',
                       'copyright': COPYRIGHT, 'licenseDescription': LICENSE,
                       'licenseInfoURL': 'https://openfontlicense.org',
                       icon_font.SOURCES_NAME_ID: icon_font.sources_digest(root)})
    fb.setupOS2(sTypoAscender=ASCENT, sTypoDescender=-DESCENT, usWinAscent=ASCENT, usWinDescent=DESCENT,
                achVendID='RMCS', fsType=0)
    fb.setupPost()
    fb.setupHead(unitsPerEm=1000, created=0, modified=0)
    out = io.BytesIO()
    fb.save(out)
    # What it holds, for tools/icon_font.py; saving again changes only the
    # name table and head's checksum.
    fb.font['name'].setName(icon_font.payload_digest(out.getvalue()), icon_font.PAYLOAD_NAME_ID, 3, 1, 0x409)
    out = io.BytesIO()
    fb.save(out)
    return out.getvalue()


def main(argv):
    check = '--check' in argv
    args = [a for a in argv if a != '--check']
    root = Path(args[0] if args else Path(__file__).resolve().parent.parent).resolve()
    data = build(root)
    target = root / 'assets/fonts/RomacsIcons.ttf'
    if check:
        if not target.exists() or target.read_bytes() != data:
            print(f'{target} is stale: run scripts/build-icon-font.py', file=sys.stderr)
            return 1
        return 0
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(data)
    print(f'wrote {target} ({len(data)} bytes)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main(sys.argv[1:]))
