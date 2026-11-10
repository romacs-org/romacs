#!/usr/bin/env python3
"""Turn ladder records + GNU's trace into verdicts and the ranked fix list.

    judge.py RUN_DIR TRACE.json FRONTIER.json VM_MISSING.txt DEFUN_FILES.txt OUT.md

Verdicts come only from what the VM itself reported in the real GUI run
(stdout command replies, cursor publication, painted rows from the scene dump)
compared with GNU 32.0.50 doing the same keys on the same fixture
(gnu-oracle.el). The ranking comes from GNU's interactive trace (gnu-trace.el):
for every step, which C primitives VM2 lacks were called while GNU did it, plus
the measured Lisp load frontier of the file that defines the step's command.
"""
import collections, json, re, sys
from pathlib import Path

run, trace_p, frontier_p, missing_p, defuns_p, out_p = sys.argv[1:7]
rec = {m: json.loads((Path(run) / m / 'record.json').read_text()) for m in ('default', 'gnu-lisp', 'loadup')
       if (Path(run) / m / 'record.json').exists()}
# 'loadup' (--loadup: loadup.el + stand-ins) is judged when recorded.
MODES = [m for m in ('default', 'gnu-lisp', 'loadup') if m == 'default' or m == 'gnu-lisp' or m in rec]
# Modes with an evaluator (the C-x C-e stand-in), so the fixture can be loaded
# and every state is compared with GNU's.
EVAL_MODES = ('default', 'loadup')
# The GUI's own loadup.el table, from whichever recorded launch ran loadup
# (--loadup, else the default if it ever runs loadup again):
# lisp/FILE.el -> (site, line, file, status, forms, detail).
gui_loadup = {}
loadup_mode = next((m for m in ('loadup', 'default') if rec.get(m, {}).get('launch', {}).get('loadup')), None)
for line in rec.get(loadup_mode, {}).get('launch', {}).get('loadup', []) if loadup_mode else []:
    cols = line.split('\t')
    if len(cols) >= 5 and cols[0].isdigit():
        cols += [''] * (6 - len(cols))
        name = cols[2] if cols[2].endswith('.el') else cols[2] + '.el'
        gui_loadup.setdefault('lisp/' + name, cols)
trace = json.loads(Path(trace_p).read_text())
frontier = {r['file']: r for r in json.loads(Path(frontier_p).read_text())}
missing = set(Path(missing_p).read_text().split())
cfile = {}
for line in Path(defuns_p).read_text().splitlines():
    f, n = line.split(' ', 1)
    cfile.setdefault(n, f)

# (id, rung, label, GUI action specs, trace step, GNU command, GNU file)
STEPS = [
    ('launch', 'launch', 'romacs starts, window maps, font renders', [], None, None, None),
    ('launch-cursor', 'launch', 'a cursor is painted at point', [], None, None, None),
    ('visit', 'visit', 'C-x C-f opens notes.org', ['C-x C-f'], 'visit', 'find-file', 'lisp/files.el'),
    ('look', 'look', 'mode line: real buffer name and position', ['RESET'], 'look', 'mode-line-format', 'lisp/bindings.el'),
    ('move-char', 'move', 'C-f C-b', ['C-f', 'C-b', '20*C-f'], 'move-char', 'forward-char', None),
    ('move-line', 'move', 'C-n C-p (goal column)', ['C-n', 'C-p'], 'move-line', 'next-line', 'lisp/simple.el'),
    ('move-bol-eol', 'move', 'C-a C-e', ['C-a', 'C-e'], 'move-bol-eol', 'org-beginning-of-line', 'lisp/org/org.el'),
    ('move-buffer', 'move', 'M-< M->', ['M-<', 'M->'], 'move-buffer', 'beginning-of-buffer', 'lisp/simple.el'),
    ('scroll-page', 'scroll', 'C-v M-v', ['C-v', 'M-v'], 'scroll-page', 'scroll-up-command', 'lisp/window.el'),
    ('recenter', 'scroll', 'C-l', ['C-l'], 'recenter', 'recenter-top-bottom', 'lisp/window.el'),
    ('point-visible', 'scroll', 'point stays visible (130 x C-n)', ['130*C-n'], 'point-visible', None, None),
    ('self-insert', 'edit', 'typing inserts', ['T: edited'], 'self-insert', 'self-insert-command', None),
    ('delete-backward', 'edit', 'DEL', ['DEL'], 'delete-backward', 'org-delete-backward-char', 'lisp/org/org.el'),
    ('kill-line', 'edit', 'C-k', ['C-k'], 'kill-line', 'org-kill-line', 'lisp/org/org.el'),
    ('undo', 'edit', 'C-/ undo', ['C-/'], 'undo', 'undo', 'lisp/simple.el'),
    ('save', 'save', 'C-x C-s, byte-identical round trip', ['C-x C-s'], 'save', 'save-buffer', 'lisp/files.el'),
    ('split-window', 'windows', 'C-x 2, C-x 3', ['C-x 2', 'C-x 3'], 'split-window', 'split-window-below', 'lisp/window.el'),
    ('other-window', 'windows', 'C-x o', ['C-x o'], 'other-window', 'other-window', 'lisp/window.el'),
    ('delete-other-windows', 'windows', 'C-x 1', ['C-x 1'], 'delete-other-windows', 'delete-other-windows', 'lisp/window.el'),
    ('switch-buffer', 'buffers', 'C-x b', ['C-x b'], 'switch-buffer', 'switch-to-buffer', 'lisp/window.el'),
    ('list-buffers', 'buffers', 'C-x C-b', ['C-x C-b'], 'list-buffers', 'list-buffers', 'lisp/buff-menu.el'),
    ('eval-expression', 'eval', 'M-: (+ 1 2)', ['M-:'], 'eval-expression', 'eval-expression', 'lisp/simple.el'),
    ('eval-last-sexp', 'eval', 'C-x C-e on (+ 1 2)', ['C-x C-e'], 'eval-last-sexp', 'eval-last-sexp', 'lisp/progmodes/elisp-mode.el'),
    ('M-x', 'command', 'M-x with completion', ['M-x'], 'M-x-completion', 'execute-extended-command', 'lisp/simple.el'),
    ('org-cycle', 'org', 'TAB folds a heading', ['TAB'], 'org-cycle', 'org-cycle', 'lisp/org/org-cycle.el'),
    ('org-edit', 'org', 'M-RET inserts a heading', ['M-RET'], 'org-edit', 'org-meta-return', 'lisp/org/org.el'),
]
# GNU's bindings and their implementation language (symbol-file at the pin, measured by where.el).
C_COMMANDS = {'forward-char', 'backward-char', 'self-insert-command', 'delete-char'}
RUNG_OF = {s[0]: s[1] for s in STEPS}


def obs_for(r, rung, specs):
    rs = next((x for x in r['rungs'] if x['rung'] == rung), None)
    if not rs: return []
    return [o for o in rs['obs'] if o.get('action') in specs]


def reply_cmds(o):
    return [(x.get('native_command'), x.get('message'), (x.get('signal') or {}).get('condition')) for x in o.get('replies', [])
            if 'native_command' in x or 'native_error' in x]


def rows_of(o, role='Text'):
    return [x for x in o.get('rows', []) if x[0] == role]


def selected_window_rows(o):
    """Rows of the window holding the active cursor (x of its clip)."""
    act = [c for c in o.get('cursors', []) if c['active']]
    text = rows_of(o)
    if not text: return []
    xs = sorted({x[1] for x in text})
    x0 = xs[0]
    if act:
        cx = act[0]['x']
        x0 = max([x for x in xs if x <= cx] or [xs[0]])
    return [x[3] for x in sorted(text, key=lambda x: x[2]) if x[1] == x0]


SEV = {'WORKS': 0, 'UNTESTED': 1, 'STAND-IN': 2, 'BLOCKED': 3}


def worse(a, b):
    return a if SEV[a] >= SEV[b] else b


def judge(mode, sid):
    r = rec.get(mode)
    if r is None: return ('UNTESTED', 'no record')
    step = next(s for s in STEPS if s[0] == sid)
    _, rung, _, specs, _, gnucmd, _ = step
    if sid == 'launch':
        L = r['launch']
        font = [f for f in L['font'] if 'font family=' in f]
        painted = [x for x in L['rows'] if x[0] == 'Text']
        unresolved = [p.get('unresolved_face_text_total') for p in L.get('paint', [])]
        files = L.get('mapped_font_files', [])
        iosevka = [f for f in files if 'iosevka' in f.lower()]
        ev = (f"ready={L['ready']}; {font[0] if font else 'NO font line'}; mapped font files "
              f"{[f.rsplit('/', 1)[-1] for f in files]}; {len(painted)} text rows painted; "
              f"unresolved_face_text_total={unresolved}")
        ok = bool(L['ready']) and bool(painted) and bool(font) and bool(iosevka) and unresolved and unresolved[0] == 0
        return ('WORKS' if ok else 'BLOCKED', ev)
    if sid == 'launch-cursor':
        L = r['launch']
        act = [c for c in L.get('cursors', []) if c['active']]
        pts = L.get('points', [])
        ev = f"cursor publication {L.get('cursors')}; points {pts}"
        if act:
            return ('WORKS', ev + '; the active cursor is painted in the selected window')
        if not [x for x in L.get('rows', []) if x[0] == 'ModeLine']:
            return ('BLOCKED', ev + '; NO cursor is painted for any window, and no mode line either: '
                    'layout draws a cursor only in a window that has a mode line (romacs-layout '
                    'layout_frame_inner); traced for --gnu-lisp to a void mode-line-format, '
                    'see evidence/loadup/LOADUP.md')
        return ('BLOCKED', ev + '; NO cursor is painted for any window (cause not traced)')
    obs = obs_for(r, rung, specs)
    if sid == 'look':
        rs = next(x for x in r['rungs'] if x['rung'] == 'look')
        last = rs['obs'][-1] if rs['obs'] else {}
        if mode not in EVAL_MODES:
            # no evaluator: look at what the launch frame shows
            last = {'rows': r['launch']['rows']}
        ml = [x[3] for x in last.get('rows', []) if x[0] == 'ModeLine']
        if not ml: return ('BLOCKED', 'no mode line is painted at all (scene dump has no ModeLine row)')
        pos = any(re.search(r'(All|Top|Bot|\d+%)', m) and re.search(r'L\d+', m) for m in ml)
        name = any('notes.org' in m for m in ml)
        ev = f'painted mode lines {ml!r}; position shown={pos}; visited file name shown={name}'
        if any('[stand-in' in m for m in ml):
            return ('STAND-IN', ev + "; romacs's labelled stand-in format (startup.rs STAND_IN_MODE_LINE), not GNU's mode-line-format")
        if pos and name: return ('WORKS', ev)
        return ('STAND-IN', ev + "; a mode line without GNU's position and visited file name")
    if not obs:
        return ('UNTESTED', 'no observation recorded')
    notes, verdict = [], 'WORKS'
    for o in obs:
        cmds = reply_cmds(o)
        if not cmds:
            notes.append(f"{o['action']}: no reply"); verdict = 'BLOCKED'; continue
        names = {c[0] for c in cmds}
        bad = [c for c in cmds if c[0] is None or (c[2] and c[2] not in ('end-of-buffer', 'beginning-of-buffer'))]
        if bad:
            notes.append(f"{o['action']}: {bad[0][1]!r}" + (f" (command {bad[0][0]}, signal {bad[0][2]})" if bad[0][0] else ''))
            verdict = 'BLOCKED'; continue
        gnu = (o.get('gnu') or {}) if mode in EVAL_MODES else {}
        want = (o.get('gnu') or {}).get('command') or gnucmd
        pts = [p['point_byte'] for p in o.get('points', [])]
        detail = f"{o['action']}: ran {sorted(names)}"
        if mode not in EVAL_MODES:
            if sid == 'self-insert':
                hit = [x[3] for x in rows_of(o) if x[3].endswith(' edited')]
                detail += f"; painted row ending ' edited': {hit[:1]!r}"
                if not hit: verdict = 'BLOCKED'
            elif not (want and names != {want}):
                detail += '; state NOT compared with GNU (no evaluator under --gnu-lisp, so the fixture cannot be put in a buffer)'
                verdict = worse(verdict, 'UNTESTED')
        if gnu.get('byte') is not None and pts:
            detail += f"; point byte {pts[0]} (GNU {gnu['byte']})"
            if pts[0] != gnu['byte']:
                verdict = worse(verdict, 'STAND-IN'); detail += ' MISMATCH'
        if gnu.get('line-text') is not None:
            win = selected_window_rows(o)
            line = gnu.get('line')
            if line and win and line <= len(win):
                got = win[line - 1]
                detail += f"; painted line {line} {got!r} (GNU {gnu['line-text']!r})"
                if got != gnu['line-text']:
                    verdict = worse(verdict, 'STAND-IN'); detail += ' MISMATCH'
        # A reply that names GNU's command is GNU's only if the launch bound
        # GNU's definition: when the launch reports a romacs stand-in for that
        # command, GNU's definition did not load, so the name cannot be GNU's.
        bound = {b[1]: b[2] for b in r.get('launch', {}).get('bindings', []) if len(b) == 3}
        if want in names and want in bound and bound[want] != want:
            detail += f"; the launch bound romacs's {bound[want]} for GNU's {want}, so a reply named {want} is not GNU's"
            verdict = worse(verdict, 'STAND-IN')
        if sid == 'self-insert':
            want = 'self-insert-command'
        if want and names != {want} and not (sid == 'self-insert' and names == {'self-insert-command'}):
            detail += f"; GNU runs {want}"
            verdict = worse(verdict, 'STAND-IN')
        notes.append(detail)
    if sid == 'point-visible' and verdict != 'BLOCKED':
        o = obs[-1]
        act = [c for c in o.get('cursors', []) if c['active']]
        win = selected_window_rows(o)
        pts = [p['point_byte'] for p in o.get('points', [])]
        moved = bool(win) and not win[0].startswith('#+title')
        notes.append(f"after 130 C-n: point byte {pts[:1]}, active cursor published={bool(act)}, first painted row {win[:1]!r}")
        if not act or not moved:
            verdict = 'BLOCKED'
            notes.append('point left the window: window-start never moved and no cursor is painted for the selected window')
    if sid == 'eval-last-sexp' and verdict != 'BLOCKED':
        o = obs[-1]
        echo = [x[3] for x in o.get('rows', []) if x[0] == 'Text' and x[2] > max([y[2] for y in o['rows'] if y[0] == 'ModeLine'] or [0])]
        notes.append(f'echo area shows {echo!r}; GNU eval-last-sexp echoes "3 (#o3, #x3, ?\\C-c)"')
        notes.append("command is romacs's bridge (crates/romacs-vm-gui/src/eval_last_sexp.rs), not elisp-mode.el")
        verdict = worse(verdict, 'STAND-IN')
    if rung == 'windows':
        verdict = worse(verdict, 'STAND-IN')
    return (verdict, ' | '.join(notes))


# ---- GNU trace: missing primitives per step ----
hits = collections.OrderedDict()
for s, p, n in trace['hits']:
    if p in ('execute-kbd-macro', 'redisplay'):  # called by the trace harness itself
        continue
    hits.setdefault(s, collections.OrderedDict())
    hits[s][p] = hits[s].get(p, 0) + n

lines = []
P = lines.append
verdicts = {}
for mode in MODES:
    for s in STEPS:
        verdicts[(mode, s[0])] = judge(mode, s[0])

counts = {m: collections.Counter(verdicts[(m, s[0])][0] for s in STEPS) for m in MODES}
heads = {m: rec[m]['head'][:8] for m in rec}
P(f"# Journey ladder -- generated by judge.py\n")
P(f"default (`tools/romacs`) on master `{heads.get('default')}`: " + ', '.join(f'{k} {v}' for k, v in sorted(counts['default'].items())))
P(f"`--gnu-lisp` (`tools/romacs --gnu-lisp`) on master `{heads.get('gnu-lisp')}`: " + ', '.join(f'{k} {v}' for k, v in sorted(counts['gnu-lisp'].items())))
if 'loadup' in MODES:
    P(f"`--loadup` (`tools/romacs --loadup`) on master `{heads.get('loadup')}`: " + ', '.join(f'{k} {v}' for k, v in sorted(counts['loadup'].items())))
P('')
extra = ' --loadup |' if 'loadup' in MODES else ''
P('| # | rung | step | default | --gnu-lisp |' + extra + ' GNU runs (file) |')
P('|---|---|---|---|---|' + ('---|' if extra else '') + '---|')
for i, s in enumerate(STEPS, 1):
    sid, rung, label, _, _, cmd, f = s
    gcell = f"`{cmd}` ({f or 'C'})" if cmd else ('C: xdisp.c redisplay' if sid == 'point-visible' else '--')
    ws = f" **{verdicts[('loadup', sid)][0]}** |" if extra else ''
    P(f"| {i} | {rung} | {label} | **{verdicts[('default', sid)][0]}** | **{verdicts[('gnu-lisp', sid)][0]}** |{ws} {gcell} |")
P('')
P('## Evidence per step\n')
for i, s in enumerate(STEPS, 1):
    sid = s[0]
    P(f"**{i}. {s[1]} / {s[2]}**")
    for mode in MODES:
        v, ev = verdicts[(mode, sid)]
        P(f"- {mode}: {v} -- {ev}")
    gl = gui_loadup.get(s[6]) if s[6] else None
    if gl:
        P(f"- the GUI's own loadup.el run (--{loadup_mode}), site {gl[0]} `{gl[2]}`: {gl[3]} {gl[4]} {gl[5][:220]}")
    if s[6]:
        fr = frontier.get(s[6])
        if fr:
            P(f"- GNU's `{s[5]}` lives in `{s[6]}`; loaded on top of the warm chain it stops at form {fr.get('forms_ok', 0) + 1}: `{(fr.get('error') or '').split(': ', 1)[-1][-160:].strip()}`".replace('\n', ' '))
    t = hits.get(s[4] or '', {})
    if t:
        P(f"- GNU calls {len(t)} C primitives VM2 lacks while doing this: " + ', '.join(f'`{p}`' for p in list(t)[:40]) + (' ...' if len(t) > 40 else ''))
    P('')


# ---- ranking ----
# Lisp side: the measured first failure of the file defining the step's command.
lisp_need = collections.defaultdict(set)
for s in STEPS:
    sid, f = s[0], s[6]
    if verdicts[('default', sid)][0] == 'WORKS' or not f:
        continue
    gl = gui_loadup.get(f)
    if gl is None:
        lisp_need['the default launch runs no GNU Lisp, and no recorded launch ran loadup.el'].add(sid)
    elif gl[3] != 'COMPLETE':
        lisp_need[f'`{f}` under loadup.el (--{loadup_mode}): {gl[3]} {gl[4]}'].add(sid)
    fr = frontier.get(f)
    if not fr or fr.get('ok'):
        continue
    err = fr.get('error') or ''
    if 'cl-lib.el: line 170 form 10' in err or "(require 'cl-lib)" in err:
        lisp_need['cl-lib.el loads (stops at form 10, line 170: `gv-define-expander` is not fbound)'].add(sid)
    elif 'defgroup' in err:
        lisp_need['custom.el runs before files.el (files.el form 3 `defgroup` is not fbound)'].add(sid)
    elif 'propertize' in err:
        lisp_need['`propertize` (editfns.c) for bindings.el form 16'].add(sid)
    elif 'vconcat' in err or 'key-parse' in err:
        lisp_need['`vconcat` (fns.c) for window.el form 424'].add(sid)
    else:
        lisp_need[f'{f} first failure: {err[-80:]}'].add(sid)
if verdicts[('default', 'point-visible')][0] != 'WORKS':
    lisp_need['point-follows redisplay (C: xdisp.c redisplay_window; lane gui/tierb)'].add('point-visible')

# C side: VM2-missing primitives GNU calls while doing each non-WORKS step.
prim_steps = collections.defaultdict(set)
for s in STEPS:
    if verdicts[('default', s[0])][0] == 'WORKS':
        continue
    for p in hits.get(s[4] or '', {}):
        if p in missing:
            prim_steps[p].add(s[0])
area = collections.defaultdict(lambda: {'steps': set(), 'prims': set()})
for p, v in prim_steps.items():
    a = area[cfile.get(p, '?')]
    a['steps'] |= v
    a['prims'].add(p)

P('## Ranked fix list\n')
P('"Steps" = journey steps (not WORKS today on the default launcher) whose GNU implementation '
  'needs the item: the file that defines the command stops loading there, or GNU calls the '
  'primitive while doing the step. A step opens only when ALL of its items are done, so this '
  'counts the steps an item is on the critical path of, not the steps it opens alone.\n')
P('### Lisp loading (measured first failures)\n')
P('| rank | item | steps | which |')
P('|---|---|---|---|')
for i, (k, v) in enumerate(sorted(lisp_need.items(), key=lambda kv: (-len(kv[1]), kv[0])), 1):
    P(f"| L{i} | {k} | {len(v)} | {', '.join(s[0] for s in STEPS if s[0] in v)} |")
P('')
P('### C primitives VM2 lacks, grouped by GNU source file\n')
P('| rank | C file | steps | missing primitives GNU called (most-used first) |')
P('|---|---|---|---|')
for i, (k, a) in enumerate(sorted(area.items(), key=lambda kv: (-len(kv[1]['steps']), len(kv[1]['prims']), kv[0])), 1):
    ps = sorted(a['prims'], key=lambda p: (-len(prim_steps[p]), p))
    P(f"| C{i} | {k} | {len(a['steps'])} | {len(ps)}: " + ', '.join(f'`{p}`({len(prim_steps[p])})' for p in ps[:14]) + (' ...' if len(ps) > 14 else '') + ' |')
P(f"\n{len(prim_steps)} distinct VM2-missing C primitives are called by GNU across the non-WORKS steps "
  f"(lower bound: see method).\n")

# The worklist in PLAN.md's shape: per step, primitives in first-use order.
tsv = ['step\torder\tprimitive\tc_file\tcalls\tvm2_fbound']
for s in STEPS:
    for i, (p, n) in enumerate(hits.get(s[4] or '', {}).items(), 1):
        tsv.append(f"{s[0]}\t{i}\t{p}\t{cfile.get(p, '?')}\t{n}\t{'no' if p in missing else 'yes'}")
Path(out_p).with_suffix('.tsv').write_text('\n'.join(tsv) + '\n')
Path(out_p).write_text('\n'.join(lines) + '\n')
print('\n'.join(lines[:34]))
