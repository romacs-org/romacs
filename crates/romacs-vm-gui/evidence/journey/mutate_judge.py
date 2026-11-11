#!/usr/bin/env python3
"""Prove judge.py can fail: mutate a real record the way a plausible-but-wrong
GUI would behave and check that the verdict moves.

    mutate_judge.py RUN_DIR TRACE FRONTIER MISSING DEFUNS

Each mutant copies RUN_DIR's two records, edits one observation, re-judges,
and must change the named step's default-mode verdict. A mutant that leaves
the verdict unchanged is a judge that cannot see that defect.
"""
import copy, json, shutil, subprocess, sys, tempfile
from pathlib import Path

run, trace, frontier, missing, defuns = sys.argv[1:6]
HERE = Path(__file__).resolve().parent
base = {m: json.loads((Path(run) / m / 'record.json').read_text()) for m in ('default', 'lisp/gnu')}


def obs(r, rung, action, nth=0):
    rs = next(x for x in r['rungs'] if x['rung'] == rung)
    return [o for o in rs['obs'] if o['action'] == action][nth]


def verdicts(recs):
    with tempfile.TemporaryDirectory() as d:
        for m, r in recs.items():
            (Path(d) / m).mkdir()
            (Path(d) / m / 'record.json').write_text(json.dumps(r))
        out = Path(d) / 'v.md'
        subprocess.run([sys.executable, str(HERE / 'judge.py'), d, trace, frontier, missing, defuns, str(out)],
                       check=True, capture_output=True)
        v = {}
        for line in out.read_text().splitlines():
            if line.startswith('| ') and line.count('|') == 7 and '**' in line:
                cells = [c.strip() for c in line.strip('|').split('|')]
                v[cells[2]] = cells[3].strip('*')
        return v


def m_point(r):          # C-f lands one byte short (a char/byte confusion)
    obs(r, 'move', 'C-f')['points'][0]['point_byte'] -= 1
def m_command(r):        # C-f runs a romacs stand-in instead of GNU's forward-char
    for x in obs(r, 'move', 'C-f')['replies']: x['native_command'] = 'romacs-gui-forward-char'
def m_text(r):           # typed text painted wrong (a dropped character)
    o = obs(r, 'edit', 'T: edited')
    for row in o['rows']:
        if row[3] == '* First heading edited': row[3] = '* First heading edted'
def m_font(r):           # the requested font never loaded (fallback face)
    r['launch']['mapped_font_files'] = ['/usr/share/fonts/TTF/DejaVuSans.ttf']
def m_cursor(r):         # no cursor painted
    r['launch']['cursors'] = []
def m_scroll_fixed(r):   # the redisplay fix: window followed point, cursor painted
    o = obs(r, 'scroll', '130*C-n')
    o['cursors'] = [{'window': 1, 'x': 0.0, 'y': 1200.0, 'active': True, 'color': '#9099d9'}]
    for row in o['rows']:
        if row[0] == 'Text' and row[1] == 0.0 and row[2] == 0.0: row[3] = 'Line 83'
def m_undefined(r):      # the self-insert key reports "Key is undefined"
    for x in obs(r, 'edit', 'T: edited')['replies']:
        x['native_command'] = None; x['message'] = 'Key is undefined'

def m_label_upgrade(r):  # a stand-in answers under GNU's name (the old eval bridge did)
    r['launch']['bindings'] = [['[14]', 'next-line', 'romacs-gui-next-line'], ['[16]', 'previous-line', 'romacs-gui-previous-line']]
    for a in ('C-n', 'C-p'):
        for o in [o for x in r['rungs'] if x['rung'] == 'move' for o in x['obs'] if o['action'] == a]:
            for x in o['replies']:
                if x.get('native_command') in ('romacs-gui-next-line', 'romacs-gui-previous-line'):
                    x['native_command'] = x['native_command'].replace('romacs-gui-', '')

# Mutants that must NOT move the verdict: a plausible-but-wrong record that
# would upgrade a label if the judge trusted a command's name.
STAY = [('label upgrade', m_label_upgrade, 'C-n C-p (goal column)')]

MUTANTS = [('point byte', m_point, 'C-f C-b'), ('command name', m_command, 'C-f C-b'),
           ('painted text', m_text, 'typing inserts'), ('font fallback', m_font, 'romacs starts, window maps, font renders'),
           ('no cursor', m_cursor, 'a cursor is painted at point'),
           ('scroll fixed', m_scroll_fixed, 'point stays visible (130 x C-n)'),
           ('key undefined', m_undefined, 'typing inserts')]
v0 = verdicts(base)
fail = 0
for name, fn, step in MUTANTS:
    recs = copy.deepcopy(base)
    fn(recs['default'])
    v1 = verdicts(recs)
    moved = v1.get(step) != v0.get(step)
    fail += not moved
    print(f"{'KILLED ' if moved else 'SURVIVED'} {name:14s} step {step!r}: {v0.get(step)} -> {v1.get(step)}")
for name, fn, step in STAY:
    recs = copy.deepcopy(base)
    fn(recs['default'])
    applied = recs['default'] != base['default']
    v1 = verdicts(recs)
    held = applied and v1.get(step) == v0.get(step) and v0.get(step) != 'WORKS'
    fail += not held
    print(f"{'HELD   ' if held else 'UPGRADED'} {name:14s} step {step!r}: {v0.get(step)} -> {v1.get(step)} (mutant applied: {applied})")
print('RESULT', 'all mutants killed' if not fail else f'{fail} survived')
sys.exit(1 if fail else 0)
