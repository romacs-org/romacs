#!/usr/bin/env python3
"""Write assets/icons/NAME.svg from the upstream glyphs assets/icons/sources.txt names.

Each line of sources.txt gives an icon NAME, a Nerd Fonts glyph name, its
codepoint and its glyph set. This reads that codepoint's outline from
Symbols Nerd Font Mono 3.4.0 (the font sources.txt identifies by its
SHA-256; refused otherwise) and writes it as the one <path> of NAME.svg:
scaled uniformly to fit a box 600 wide and 700 tall, centred on the
500-wide cell of the 500-by-1000 box scripts/build-icon-font.py reads (a
wide glyph overhangs its cell by at most a tenth of it on each side, as
nerd-icons' do), the text baseline at y=800 and the box's middle 315
above it. Contours keep
TrueType's direction once the build reverses them.

The import is a one-time step per glyph change: the SVGs are checked in,
and the font is built from them alone (scripts/build-icon-font.py), so no
build needs the upstream font. Needs fontTools (pinned through the flake's
nixpkgs: `nix build --inputs-from . nixpkgs#python3.pkgs.fonttools`).

usage: import-icon-glyphs.py NFM.ttf [ROOT]
"""
import hashlib
import re
import sys
from pathlib import Path

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.reverseContourPen import ReverseContourPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont

WIDTH, HEIGHT, MIDDLE, BASELINE, BOX = 600, 700, 315, 800, 500


def sources(root):
    text = (root / 'assets/icons/sources.txt').read_text()
    digest = re.search(r'SHA-256\s*#?\s*([0-9a-f]{64})', text.replace('\n#', ' ')).group(1)
    rows = [line.split() for line in text.splitlines() if line.strip() and not line.startswith('#')]
    return digest, [(name, int(point.removeprefix('U+'), 16)) for name, _, point, _ in rows]


def svg(glyphs, glyph):
    bounds = BoundsPen(glyphs)
    glyphs[glyph].draw(bounds)
    x0, y0, x1, y1 = bounds.bounds
    scale = min(WIDTH / (x1 - x0), HEIGHT / (y1 - y0))
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    pen = SVGPathPen(glyphs, ntos=lambda v: f'{v:.1f}')
    # Font units, y up, to the SVG box, y down: centred on the text band.
    transform = (scale, 0, 0, -scale, BOX / 2 - scale * cx, BASELINE - MIDDLE + scale * cy)
    glyphs[glyph].draw(TransformPen(ReverseContourPen(pen), transform))
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {BOX} 1000" width="{BOX}" height="1000">\n'
            f'  <path fill="currentColor" d="{pen.getCommands()}"/>\n</svg>\n')


def main(argv):
    if not argv:
        raise SystemExit(__doc__)
    font_path = Path(argv[0])
    root = Path(argv[1] if len(argv) > 1 else Path(__file__).resolve().parent.parent).resolve()
    digest, icons = sources(root)
    if hashlib.sha256(font_path.read_bytes()).hexdigest() != digest:
        raise SystemExit(f'{font_path} is not the font sources.txt names (SHA-256 {digest})')
    font = TTFont(font_path)
    cmap, glyphs = font.getBestCmap(), font.getGlyphSet()
    for name, point in icons:
        if point not in cmap:
            raise SystemExit(f'{name}: U+{point:04X} is not in {font_path}')
        (root / f'assets/icons/{name}.svg').write_text(svg(glyphs, cmap[point]))
    print(f'wrote {len(icons)} icons')
    return 0


if __name__ == '__main__':
    raise SystemExit(main(sys.argv[1:]))
