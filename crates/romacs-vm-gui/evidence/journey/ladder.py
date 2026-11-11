#!/usr/bin/env python3
"""A first session as an executable ladder, measured on the real GUI.

    ladder.py OUT_DIR [--mode default|lisp/gnu|loadup] [--binary PATH]

Modes are launch flags of the binary under test: default (no flag), lisp/gnu
(--gnu-lisp, loadup.el alone) and loadup (--loadup, loadup.el + stand-ins).
The default is VM2's own workspace with no GNU Lisp.

Every rung is a list of actions; each action is what a person does (a chord or
typed text) paired with the `kbd' string GNU 32.0.50 is given for the same
action. The run records, per action, the VM's OWN reply (which command ran,
what it signalled), the cursor publication (point byte per window) and the
painted rows from the scene dump; GNU's answer comes from gnu-oracle.el on a
fresh visit of the same fixture. Verdicts are assigned by judge() below from
those records only -- never from a screenshot.

The visit rung cannot work today, so later rungs start from a MEASUREMENT
WORKAROUND, labelled as such: the fixture's text is put in the selected
window's buffer by evaluating (insert-file-contents ...) with C-x C-e. That
workaround is the "reset" action; it is not a journey step and earns no credit.
"""
import argparse, hashlib, json, re, subprocess, sys, time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parent))
import jlib

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
FIXTURE = HERE / 'fixture' / 'notes.org'
ap = argparse.ArgumentParser()
ap.add_argument('out')
ap.add_argument('--mode', default='default', choices=['default', 'lisp/gnu', 'loadup'])
ap.add_argument('--binary', default=str(ROOT / 'target/release/romacs-vm-gui'))
args = ap.parse_args()
OUT = Path(args.out).resolve() / args.mode
OUT.mkdir(parents=True, exist_ok=True)

RESET_FORM = f'(progn (erase-buffer) (insert-file-contents "{FIXTURE}") (goto-char 1) (set-window-point (selected-window) 1))'
R = ('RESET', 'RESET')
# rung -> actions: (gui spec, GNU kbd or None). 'T:' types text. 'N*spec' repeats.
RUNGS = [
    ('launch', []),
    ('visit', [('C-x C-f', None)]),
    ('look', [R]),
    ('move', [R, ('C-n', 'C-n'), ('C-n', 'C-n'), ('C-n', 'C-n'), ('C-f', 'C-f'), ('C-f', 'C-f'),
              ('C-e', 'C-e'), ('C-a', 'C-a'),
              # across "κόσμε" (two bytes per character): char/byte confusion shows here
              ('20*C-f', ' '.join(['C-f'] * 20)), ('C-b', 'C-b'), ('C-p', 'C-p'),
              ('M->', 'M->'), ('M-<', 'M-<')]),
    ('scroll', [R, ('C-v', 'C-v'), ('M-v', 'M-v'), ('C-l', 'C-l'), ('130*C-n', None)]),
    ('edit', [R, ('C-n', 'C-n'), ('C-e', 'C-e'), ('T: edited', 'SPC e d i t e d'),
              ('DEL', 'DEL'), ('DEL', 'DEL'), ('C-k', 'C-k'), ('C-/', 'C-/')]),
    ('save', [('C-x C-s', None)]),
    ('windows', [('C-x 2', None), ('C-x 3', None), ('C-x o', None), ('C-x 1', None)]),
    ('buffers', [('C-x b', None), ('C-x C-b', None)]),
    ('eval', [('M-:', None), ('M->', None), ('T:(+ 1 2)', None), ('C-x C-e', None)]),
    ('command', [('M-x', None)]),
    ('org', [R, ('C-n', 'C-n'), ('TAB', 'TAB'), ('M-RET', 'M-RET')]),
]
# Live primitive probe, evaluated in the GUI's own VM by C-x C-e at the end.
PROBE = ("(list (fboundp 'find-file) (fboundp 'save-buffer) (fboundp 'write-region) "
         "(fboundp 'buffer-modified-p) (fboundp 'line-beginning-position) (fboundp 'next-line) "
         "(fboundp 'undo) (fboundp 'scroll-up) (fboundp 'read-from-minibuffer) (fboundp 'buffer-list) "
         "(fboundp 'insert-file-contents) (fboundp 'org-mode))")


def gnu_oracle():
    keys = [a[1] for _, acts in RUNGS for a in acts if a[1]]
    kf = OUT / 'gnu-keys.txt'; kf.write_text('\n'.join(keys) + '\n')
    of = OUT / 'gnu-oracle.json'
    home = OUT / 'gnu-home'; home.mkdir(exist_ok=True)
    subprocess.run(['env', '-i', 'PATH=/usr/bin:/bin', f'HOME={home}', 'LANG=C.UTF-8', '/usr/bin/emacs', '-Q',
                    '--batch', '-l', str(HERE / 'gnu-oracle.el'), str(FIXTURE), str(kf), str(of)],
                   check=True, capture_output=True, text=True)
    rows = json.loads(of.read_text())
    return rows[1:]  # first row is the initial visit


def _unused_left_rows(g):
    rows = g.rows()
    sel = [k for k in rows if k[0] == 'Text']
    if not sel: return []
    x0 = min(k[1] for k in sel)
    return [rows[k] for k in sorted(sel, key=lambda k: k[2]) if k[1] == x0]


def observe(g, action, n0):
    g.settle()
    pts = g.last_points()
    cur = g.frames('cursor-publication')
    return {'action': action, 'replies': g.replies()[n0:], 'points': pts,
            'cursors': cur[-1]['cursors'] if cur else [],
            'rows': g.rows()}


def do(g, spec):
    if spec.startswith('T:'):
        g.text(spec[2:])
    elif '*' in spec:
        n, s = spec.split('*', 1)
        for _ in range(int(n)): g.chord(s)
    else:
        g.chord(spec)


def main():
    oracle = gnu_oracle()
    oi = iter(oracle)
    rec = {'mode': args.mode, 'binary': args.binary,
           'binary_sha256': hashlib.sha256(Path(args.binary).read_bytes()).hexdigest(),
           'head': subprocess.check_output(['git', '-C', str(ROOT), 'rev-parse', 'HEAD'], text=True).strip(),
           'fixture_sha256': hashlib.sha256(FIXTURE.read_bytes()).hexdigest(), 'rungs': []}
    g = jlib.Gui(OUT, args.binary, args.mode)
    try:
        g.settle()
        try:
            maps = Path(f'/proc/{g.pid}/maps').read_text()
            fonts = sorted({l.split()[-1] for l in maps.splitlines() if re.search(r'\.(ttf|otf|ttc)$', l)})
        except OSError as e:
            fonts = [f'unreadable: {e}']
        cur = g.frames('cursor-publication')
        rec['launch'] = {'ready': g.replies()[:1], 'cmd': g.cmd, 'mapped_font_files': fonts,
                         'cursors': cur[-1]['cursors'] if cur else [],
                         'font': [l for l in g.log.read_text().splitlines() if l.startswith('romacs:')],
                         # The GUI's own loadup.el table (site, line, file, status, forms, detail).
                         'loadup': [l.split(': ', 1)[1] for l in g.log.read_text().splitlines() if l.startswith('romacs-loadup: ')],
                         # Per journey key: (key, GNU's command, the command actually bound).
                         'bindings': [l.split(': ', 1)[1].split('\t') for l in g.log.read_text().splitlines() if l.startswith('romacs-binding: ')],
                         'paint': g.frames('paint')[:1], 'rows': g.rows(), 'points': g.last_points()}
        g.shot('launch')
        for rung, actions in RUNGS:
            obs = []
            for gui, gnu in actions:
                n0 = len(g.replies())
                if gui == 'RESET':
                    if args.mode == 'lisp/gnu':
                        next(oi); obs.append({'action': 'RESET', 'skipped': 'no evaluator under --gnu-lisp', 'rows': [], 'replies': [], 'points': [], 'cursors': []}); continue
                    g.text(RESET_FORM); g.chord('C-x C-e')
                else:
                    do(g, gui)
                o = observe(g, gui, n0)
                if gnu: o['gnu'] = next(oi)
                obs.append(o)
            g.shot(rung)
            rec['rungs'].append({'rung': rung, 'obs': obs})
            (OUT / 'record.json').write_text(json.dumps(rec, indent=1, ensure_ascii=False))
        if args.mode != 'lisp/gnu':
            n0 = len(g.replies())
            g.chord('M->'); g.text(PROBE); g.chord('C-x C-e')
            rec['probe'] = observe(g, 'probe', n0)
            g.shot('probe')
        rec['refocus'] = getattr(g, 'refocus', 0)
    finally:
        (OUT / 'record.json').write_text(json.dumps(rec, indent=1, ensure_ascii=False))
        g.close()
    print('record', OUT / 'record.json')


main()
