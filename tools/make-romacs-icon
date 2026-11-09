#!/usr/bin/env python3
"""Write assets/icons/romacs.svg, Romacs's R icon, from the Romacs mark.

The source is the red R of crates/romacs-display-runtime/assets/window-icon.svg
(its blue crescent left out: an icon is one colour).  The R stands on the
icon font's baseline at the cap height of the text it sits beside
(Iosevka's, 735 of 1000 units), centred on the 500-unit advance, and is
made heavier by STROKE units (its outline united with a stroke of it), so
its hairlines survive a status bar's size as the mark's do at its own.

Needs fontTools' companion skia-pathops (the build's Nix inputs:
nix build --inputs-from . nixpkgs#python3Packages.skia-pathops).

usage: make-romacs-icon.py [ROOT]
"""
import re
import sys
from pathlib import Path

import pathops
from fontTools.pens.svgPathPen import SVGPathPen

ROOT = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parents[1])
MARK = ROOT / 'crates/romacs-display-runtime/assets/window-icon.svg'
OUT = ROOT / 'assets/icons/romacs.svg'
BASELINE, CAP, CENTRE, STROKE = 800, 735, 250, 28


def red_r(svg):
    """The d of the mark's red path."""
    return re.search(r'<path fill="#ff5f59"[^>]* d="([^"]+)"', svg).group(1)


def parse(d):
    """D's absolute M/C/L/Z commands as (COMMAND, [(X, Y)...])."""
    out = []
    for command, args in re.findall(r'([MCLZ])([^MCLZ]*)', d):
        nums = [float(n) for n in re.findall(r'-?\d+(?:\.\d+)?', args)]
        out.append((command, list(zip(nums[0::2], nums[1::2]))))
    return out


def main():
    commands = parse(red_r(MARK.read_text()))
    points = [p for _, pts in commands for p in pts]
    xs, ys = [x for x, _ in points], [y for _, y in points]
    scale = (CAP - STROKE) / (max(ys) - min(ys))
    place = lambda x, y: ((x - (min(xs) + max(xs)) / 2) * scale + CENTRE, (y - max(ys)) * scale + BASELINE - STROKE / 2)
    path = pathops.Path()
    for command, pts in commands:
        pts = [place(*p) for p in pts]
        if command == 'M':
            path.moveTo(*pts[0])
        elif command == 'L':
            path.lineTo(*pts[0])
        elif command == 'C':
            path.cubicTo(*pts[0], *pts[1], *pts[2])
        else:
            path.close()
    stroke = pathops.Path(path)
    stroke.stroke(STROKE, pathops.LineCap.ROUND_CAP, pathops.LineJoin.ROUND_JOIN, 4)
    # Round joins are conics, which the union and TrueType do not take.
    stroke.convertConicsToQuads()
    heavy = pathops.op(path, stroke, pathops.PathOp.UNION)
    pen = SVGPathPen(None, ntos=lambda n: f'{n:.1f}')
    heavy.draw(pen)
    OUT.write_text('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 500 1000" width="500" height="1000">\n'
                   f'  <path fill="currentColor" d="{pen.getCommands()}"/>\n</svg>\n')
    print(f'wrote {OUT}')


if __name__ == '__main__':
    main()
