#!/usr/bin/env python3
"""Mutation check of the oracle itself: each mutant weakens one assertion or
one reading step; the crate's tests must fail on every one.

usage (from the workspace root, lane env sourced):
    python3 crates/romacs-gui-oracle/harness/mutants.py [ID ...]

Each mutant is applied in place, the suite is run, and the file is restored
from memory in a `finally`, whatever happens.  Prints one line per mutant:
KILLED with the failing tests, or SURVIVED.  Exit 1 if any survived.
"""
import re
import subprocess
import sys
from pathlib import Path

CRATE = Path(__file__).resolve().parent.parent
SRC = CRATE / 'src'

# (id, file, old, new, what it weakens)
MUTANTS = [
    ('M01', 'check.rs', 'pub const TOL_FG: u8 = 8;', 'pub const TOL_FG: u8 = 255;',
     'foreground colour never checked'),
    ('M02', 'check.rs', 'pub const TOL_BG: u8 = 2;', 'pub const TOL_BG: u8 = 255;',
     'background colour, cursor and stray never checked'),
    ('M03', 'check.rs', 'let text_ok = chars_alike(model, cell.ch, want.ch);',
     'let text_ok = true;', 'text never compared'),
    ('M04', 'check.rs', 'pub const MAX_GLYPH_OFFSET: i32 = 0;', 'pub const MAX_GLYPH_OFFSET: i32 = 1;',
     'glyph placement tolerates one pixel'),
    ('M05', 'check.rs', 'pub const MAX_GLYPH_OFFSET: i32 = 0;', 'pub const MAX_GLYPH_OFFSET: i32 = 99;',
     'glyph placement never checked'),
    ('M06', 'check.rs', 'if cell.underline != want.underline {', 'if false {',
     'underline ignored'),
    ('M07', 'check.rs', '&& style != want.style', '&& false',
     'weight and slant ignored'),
    ('M09', 'check.rs', 'if p.dist(spec.frame.bg) > TOL_BG {\n                stray += 1;',
     'if false {\n                stray += 1;', 'pixels outside the grid ignored'),
    ('M10', 'check.rs', '    if fringe == 0 {\n        return;\n    }', '    {\n        return;\n    }',
     'fringes ignored'),
    ('M11', 'check.rs', 'if exp.rows.len() > rows {', 'if false {',
     'rows that do not fit ignored'),
    ('M12', 'extract.rs', 'let todo: Vec<usize> = (0..rows * cols).filter(|&k| !masks[k].is_empty()).collect();',
     'let todo: Vec<usize> = Vec::new();', 'neighbour overhang not masked'),
    ('M13', 'glyphs.rs', 'pub const ALL: [Hint; 3] = [Hint::Light, Hint::Full, Hint::None];',
     'pub const ALL: [Hint; 1] = [Hint::Light];', 'one hinting mode only'),
    ('M14', 'extract.rs', 'if lb < lr { b } else { r }', 'if b.score > r.score { b } else { r }',
     'weight by shape instead of mass'),
    ('M15', 'extract.rs', 'v[2.min(v.len() - 1)]', '{ v.sort_unstable(); v[v.len() / 2] }',
     'foreground as a per-channel median'),
    ('M16', 'extract.rs', 'pub const BLANK_INK: u8 = 48;', 'pub const BLANK_INK: u8 = 200;',
     'faint ink counted as blank'),
    ('M17', 'extract.rs', 'pub const SEARCH: i32 = 2;', 'pub const SEARCH: i32 = 0;',
     'no placement search: offsets cannot be measured'),
    ('M18', 'extract.rs',
     'Some(b) => {\n                        s > b.score + 1e-6\n                            || ((s - b.score).abs() <= 1e-6\n                                && dx.abs() + dy.abs() < b.dx.abs()',
     'Some(b) => {\n                        s > b.score + 1e-6\n                            || ((s - b.score).abs() <= 1e-6\n                                && dx.abs() + dy.abs() > b.dx.abs()',
     'ties resolved toward the larger offset'),
    ('M19', 'pair.rs', 'if !chars_alike(model, x.ch, y.ch) {', 'if false {',
     'pair mode never compares text'),
    ('M21', 'pair.rs', 'if (x.dx, x.dy) != (y.dx, y.dy) {', 'if false {',
     'pair mode never compares glyph offsets'),
    ('M22', 'pair.rs', 'if ba.dist(bb) > TOL_BG {', 'if false {',
     'pair mode never compares fringe background'),
    ('M20', 'check.rs', '    let (bg, fg) = if cursor {\n        (spec.cursor_color, bg)\n',
     '    let (bg, fg) = if false {\n        (spec.cursor_color, bg)\n', 'cursor cell not expected'),
]


def run_suite():
    # Every test target of the crate: the unit tests guard the reader's
    # internals (placement ties), the integration suite everything else.
    p = subprocess.run(['cargo', 'test', '-p', 'romacs-gui-oracle', '--no-fail-fast',
                        '--', '--test-threads', '8'],
                       capture_output=True, text=True)
    failed = re.findall(r'^test (\S+) \.\.\. FAILED$', p.stdout, re.M)
    compiled = 'test result:' in p.stdout
    return p.returncode, compiled, failed, p.stderr[-2000:]


def main():
    only = set(sys.argv[1:])
    survived = 0
    for mid, fname, old, new, what in MUTANTS:
        if only and mid not in only:
            continue
        path = SRC / fname
        original = path.read_text()
        if original.count(old) != 1:
            print(f'{mid} STALE  pattern not found exactly once in {fname}: {what}')
            survived += 1
            continue
        try:
            path.write_text(original.replace(old, new))
            rc, compiled, failed, err = run_suite()
        finally:
            path.write_text(original)
        if not compiled:
            print(f'{mid} NOBUILD {what}\n{err}')
            survived += 1
        elif failed:
            print(f'{mid} KILLED  {what}: {len(failed)} failing: {", ".join(sorted(failed))}')
        else:
            print(f'{mid} SURVIVED {what} (exit {rc})')
            survived += 1
        sys.stdout.flush()
    return 1 if survived else 0


if __name__ == '__main__':
    sys.exit(main())
