#!/usr/bin/env python3
"""rectangle-mark-mode (C-x SPC) in pinned GNU and in Romacs GUI --loadup.

Each workflow starts a fresh pair of sessions (parity_live.py): pinned GNU
Emacs (pgtk) and `romacs-vm-gui --loadup', each in its own private headless
labwc with a private HOME/XDG/TMPDIR, both visiting the same small file of
long, short and tabbed lines (`initial-buffer-choice').  Keys go in through
wtype; with --shots each step leaves a PNG of each engine's output.

The workflows: C-x SPC, then C-n, C-f and C-b over short lines, a tab and
past the end of a line, C-p, C-x C-x round the four corners and C-g
(`motion'); M-w, C-y of the rectangle, C-w and C-y back (`kill-yank');
C-x r t over an active rectangle (`string-rectangle'); C-x SPC twice, and
C-x SPC over an active region (`toggle').

COMPARISON POLICY: as eshell_parity_live.py's -- buffers, point, text,
properties, overlays, modes, mode and header lines, windows, minibuffer,
echo area and *Messages* exactly; paths, wall-clock and geometry
normalised or reported apart -- with the snapshot's `:rect' entry adding
`rectangle-mark-mode', `mark-active', the region's beginning, end and
bounds (not while the minibuffer is selected: see RECT_STATE), the mark, point's column, the window's point crutches, the kill
ring's head and its yank handler, and `killed-rectangle'.  GNU's plain
region overlay (face `region', priority (nil . 100)) is dropped from its
overlays: Romacs draws that region natively (ledger/divergences
VM2-PREREDISPLAY1.md), so only the PNGs show it.  The rectangle's own
overlays, one a line, are compared.

usage: rect_parity_live.py --romacs BIN [--gnu EMACS] --output DIR
                           [--workflow NAME]... [--shots]
"""
import re
import subprocess
import sys
from pathlib import Path

from parity_live import (ROOT, C, CONTENTS_ARE, Harness, K, M, T, gnu_engine, inputs, lisp_string, main,
                         private_tree, romacs_engine)
import fileio_live as fio
import parity_live

TEXT = ('alpha beta gamma\n'
        'ab\n'
        '\ttab\tline here\n'
        'longer line number four\n'
        'x\n'
        'last line of text\n')

# The region's bounds while the minibuffer is selected are left out: at
# C-x r t's prompt they are columns over the preview's invisible and
# display overlays, which Romacs's `move-to-column' does not yet skip as
# GNU's does (indent.c scan_for_column), though the rectangle replaced is
# the same.
RECT_STATE = '''(list (bound-and-true-p rectangle-mark-mode) mark-active
      (and mark-active (not (minibufferp (window-buffer (selected-window))))
           (list (region-beginning) (region-end) (region-bounds)))
      (mark t) (current-column)
      (window-parameter nil 'rectangle--point-crutches)
      (car kill-ring) (and kill-ring (car (get-text-property 0 'yank-handler (car kill-ring))))
      (bound-and-true-p killed-rectangle))'''

# Romacs's key hints and evaluation chip, which GNU lacks.
ROMACS_CHROME_OFF = ("(dolist (mode '(romacs-popup-hints-mode romacs-eval-chip-mode))\n"
                     "  (when (fboundp mode) (funcall mode -1)))\n")


def init_source(engine, root):
    """The init both engines load: the same but for GNU's chrome and Romacs's
    defaults, which GNU takes as eshell_parity_live.py's GNU does."""
    gnu_setup = ('(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
                 '(setq scroll-conservatively 101)\n'
                 f'(load {lisp_string(str(ROOT / "lisp/completion/romacs-completion-defaults.el"))} nil t)\n') if engine == 'gnu' else ''
    return f''';; -*- lexical-binding: t -*-
{gnu_setup}{parity_live.GNU_COMPLETION}{ROMACS_CHROME_OFF}(setq inhibit-startup-screen t initial-scratch-message nil)
(setq initial-buffer-choice {lisp_string(str(root / 'work' / 'rect.txt'))})
(setq parity-probe-variables '(mark-active))
(set-face-attribute 'region nil :background "#3070c0")
(load {lisp_string(str(parity_live.PROBE))} nil t)
(defvar rect-parity-base 0 "Input events read when the step began.")
(advice-add 'parity-probe-step :before
            (lambda () (setq rect-parity-base num-nonmacro-input-events)))
(defun rect-parity-snapshot ()
  "The probe's snapshot and the rectangle's state."
  (let ((snapshot (parity-probe-snapshot)))
    (with-current-buffer (plist-get snapshot :buffer)
      (append snapshot (list :rect (parity-probe--plain {RECT_STATE}))))))
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''


def launch(engine, base, args):
    root, home, env = private_tree(engine, base, args.tools, f'{engine}-'.ljust(len('romacs-'), '_'))
    (root / 'work' / 'rect.txt').write_text(TEXT)
    init = init_source(engine, root)
    if engine == 'gnu':
        return gnu_engine(root, home, env, init, None, args.timeout, args.tools)
    return romacs_engine(args.romacs, root, home, env, init, None, args.timeout, args.tools, args=('--trace-scene',))


# --- workflows ----------------------------------------------------------------

def read(groups, also='t'):
    """Ready once the session has read GROUPS' events and runs no command."""
    return (f'(and (>= num-nonmacro-input-events (+ rect-parity-base {sum(inputs(g) for g in groups)}))'
            f' (null this-command) {also})')


def step(name, *groups, also='t'):
    return dict(name=name, trigger=list(groups), ready=read(groups, also))


RECT_ON = '(bound-and-true-p rectangle-mark-mode)'
INACTIVE = '(not mark-active)'
C_X_SPC = (C('x'), K('space'))
START = step('start', M('<'), C('f'), C('f'))
ON = step('rect-on', *C_X_SPC, also=RECT_ON)


def string_rectangle(text):
    """C-x r t to its prompt, then TEXT and RET."""
    prompt = [C('x'), T('r'), T('t')]
    return [step('string-prompt', *prompt, also='(active-minibuffer-window)'),
            dict(name='string', keys=[T(text)], keys_ready=CONTENTS_ARE(lisp_string(text)), trigger=[K('Return')],
                 ready=read([T(text), K('Return')], INACTIVE))]


WORKFLOWS = {
    'motion': [
        START, ON,
        step('down-short', C('n')),
        step('down-tab', C('n')),
        step('right', C('f'), C('f'), C('f')),
        step('down-past-eol', C('n'), C('n')),
        step('right-past-eol', C('f'), C('f')),
        step('left', C('b')),
        step('up', C('p')),
        step('exchange-1', C('x'), C('x')),
        step('exchange-2', C('x'), C('x')),
        step('exchange-3', C('x'), C('x')),
        step('exchange-4', C('x'), C('x')),
        step('quit', C('g'), also=INACTIVE),
    ],
    'kill-yank': [
        START, ON,
        step('extend', C('n'), C('n'), C('f'), C('f'), C('f')),
        step('copy', M('w'), also=INACTIVE),
        step('end', M('>')),
        step('yank', C('y')),
        step('again', M('<'), *C_X_SPC, C('n'), C('f'), C('f'), also=RECT_ON),
        step('kill', C('w'), also=INACTIVE),
        step('yank-back', C('y')),
    ],
    'string-rectangle': [
        START, ON,
        step('extend', C('n'), C('n'), C('f'), C('f'), C('f'), C('f')),
        *string_rectangle('ZZ'),
    ],
    'toggle': [
        START, ON,
        step('extend', C('n'), C('n'), C('n'), C('f')),
        step('rect-off', *C_X_SPC),
        step('mark', C('space'), C('n'), C('f')),
        step('rect-over-region', *C_X_SPC, also=RECT_ON),
        step('down', C('n')),
        step('quit', C('g'), also=INACTIVE),
    ],
}


# --- what Romacs draws ------------------------------------------------------------

REGION = (0x30, 0x70, 0xc0)  # the region's background, as both inits set it
# What Romacs must paint after a step: for each (row, column) of the text
# area named, whether the region's background fills that cell, past a
# line's end too, read from a screenshot (`painted').  The cell under the cursor is
# never named.
NOT, PAINTED = False, True
DRAWN = {
    # The rectangle from column 2 to column 7 of rows 0 to 4, past the ends
    # of the short rows and over the tab, and nothing beside it.
    ('motion', 'right-past-eol'): {
        **{(r, c): PAINTED for r in range(5) for c in range(2, 7)},
        **{(r, c): NOT for r in range(5) for c in (0, 1, 8, 9)},
        **{(5, c): NOT for c in range(10)}},
    # The rectangle turned off with the mark still active: the plain region
    # from row 0's column 2 to row 3's column 3.
    ('toggle', 'rect-off'): {(0, 1): NOT, **{(0, c): PAINTED for c in range(2, 16)},
                             **{(3, c): PAINTED for c in range(3)}, **{(3, c): NOT for c in range(4, 10)}},
    # C-g: nothing, the rectangle's overlays gone with it.
    ('toggle', 'quit'): {(r, c): NOT for r in range(6) for c in range(16)},
}

SHOTS = None
WORKFLOW = [None]
DRAWN_ERRORS, DRAWN_CHECKED = {}, {}
_run_step, _run_workflow, _compare = parity_live.run_step, parity_live.run_workflow, parity_live.compare


def pixels(png, tools):
    """PNG's width, height and RGB bytes."""
    ppm = subprocess.run([f'{tools}/magick', str(png), '-depth', '8', 'ppm:-'], capture_output=True, check=True,
                         timeout=60).stdout
    header = re.match(rb'P6\s+(\d+)\s+(\d+)\s+\d+\s', ppm)
    return int(header[1]), int(header[2]), ppm[header.end():]


def cell_box(engine):
    """The text area's origin and a cell's size, in pixels: column 0 of
    row 0 from the last scene Romacs submitted, rows by its text rows."""
    scene = fio.last_scene((engine.root / engine.editor_log).read_text(errors='replace'))
    rows = sorted({g['y'] for g in scene['glyphs'] if g['role'] == 'Text'})
    left = min(g['x'] for g in scene['glyphs'] if g['role'] == 'Text' and g['y'] == rows[0])
    return left, rows, engine.ask('(frame-char-width)')


def painted(png, engine, cells):
    """For each of CELLS, (row, column), whether the region's exact colour
    fills a sixth of its middle band: a glyph's ink covers the rest, and
    a cell the region leaves has none of it."""
    width, _, data = pixels(png, engine.tools)
    left, rows, column = cell_box(engine)
    pitch = rows[1] - rows[0]
    out = {}
    for row, col in cells:
        x0, y0 = int(left + col * column), int(rows[row] + pitch * .3)
        box = [(x, y) for x in range(x0 + 1, x0 + int(column) - 1) for y in range(y0, int(rows[row] + pitch * .7))]
        hits = sum(tuple(data[(y * width + x) * 3:(y * width + x) * 3 + 3]) == REGION for x, y in box)
        out[(row, col)] = hits >= .15 * len(box)
    return out


def run_step(engines, step_, perf_enabled, seed):
    result = _run_step(engines, step_, perf_enabled, seed)
    expected = DRAWN.get((WORKFLOW[0], step_['name']))
    for engine in engines:
        if not SHOTS and not (expected and engine.name == 'romacs'):
            continue
        target = (SHOTS or engine.root) / f'{WORKFLOW[0]}-{step_["name"]}-{engine.name}.png'
        subprocess.run([f'{engine.tools}/grim', str(target)], env=engine.env, check=True, timeout=30)
        if expected and engine.name == 'romacs':
            seen = painted(target, engine, expected)
            DRAWN_CHECKED.setdefault(WORKFLOW[0], []).append(step_['name'])
            DRAWN_ERRORS.setdefault(WORKFLOW[0], []).extend(
                f'{step_["name"]}: row {r} column {c} {"painted" if seen[(r, c)] else "not painted"}'
                for (r, c), want in sorted(expected.items()) if seen[(r, c)] != want)
    return result


def run_workflow(harness, name, args, base, report):
    WORKFLOW[0] = name
    return _run_workflow(harness, name, args, base, report)


def drawn(_engines):
    """The cells a step of this workflow painted otherwise than DRAWN says."""
    return {'errors': DRAWN_ERRORS.get(WORKFLOW[0], []),
            'checked': DRAWN_CHECKED.get(WORKFLOW[0], [])}


def plain_region(overlay):
    """Whether OVERLAY, as the probe reports it, is GNU's region overlay."""
    props = [str(p) for p in overlay[2]]
    return props[:2] == ['face', 'region'] and 'priority' in props and 'before-string' not in props


def compare(gnu, romacs):
    for window in parity_live.plist(gnu).get('windows') or []:
        for i in range(0, len(window) - 1, 2):
            if str(window[i]) == ':overlays':
                window[i + 1] = [o for o in window[i + 1] or [] if not plain_region(o)]
    return _compare(gnu, romacs)


parity_live.run_step, parity_live.run_workflow, parity_live.compare = run_step, run_workflow, compare

HARNESS = Harness(WORKFLOWS, launch, '(rect-parity-snapshot)', 'rect-parity-', check=drawn)


def arguments(parser):
    parser.add_argument('--shots', action='store_true', help='a PNG of each engine after every step')


if __name__ == '__main__':
    if '--shots' in sys.argv:
        SHOTS = Path(sys.argv[sys.argv.index('--output') + 1])
    sys.exit(main(HARNESS, __doc__, arguments))
