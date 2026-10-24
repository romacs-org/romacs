#!/usr/bin/env python3
"""Where GNU's Lisp for each journey command stops loading on this VM.

For every file that DEFINES a journey command in GNU (symbol-file at the pin),
load it on the census binary in the WARM regime -- loadup.el's preload chain
first, as far as it reaches -- and report the first failing form of that file
(line, form number, error text). FIRST failure only; later forms are not run.

    load_frontier.py CENSUS_BINARY ROOT OUT.json
"""
import json, subprocess, sys
from pathlib import Path

binary, root, out = sys.argv[1], Path(sys.argv[2]), Path(sys.argv[3])
CHAIN = ['emacs-lisp/debug-early', 'emacs-lisp/byte-run', 'emacs-lisp/backquote', 'subr',
         'keymap', 'version', 'widget', 'custom', 'emacs-lisp/map-ynp', 'international/mule',
         'international/mule-conf', 'env', 'format', 'bindings', 'window', 'files',
         'emacs-lisp/macroexp']
# journey file -> commands it defines that the ladder needs (GNU 32.0.50 symbol-file)
FILES = {
    'files': ['find-file', 'save-buffer', 'save-buffers-kill-terminal'],
    'simple': ['next-line', 'previous-line', 'move-beginning-of-line', 'move-end-of-line',
               'beginning-of-buffer', 'end-of-buffer', 'backward-delete-char-untabify',
               'kill-line', 'undo', 'eval-expression', 'execute-extended-command'],
    'window': ['scroll-up-command', 'scroll-down-command', 'recenter-top-bottom',
               'split-window-below', 'split-window-right', 'other-window',
               'delete-other-windows', 'switch-to-buffer'],
    'bindings': ['mode-line-format (value)'],
    'buff-menu': ['list-buffers'],
    'minibuffer': ['completing-read-default'],
    'progmodes/elisp-mode': ['eval-last-sexp'],
    'org/org': ['org-mode'],
    'org/org-cycle': ['org-cycle'],
}
lp = [root / 'gnu-lisp'] + sorted(p for p in (root / 'gnu-lisp').rglob('*') if p.is_dir())
pre = [a for n in CHAIN for a in ('--preload', str(root / 'gnu-lisp' / f'{n}.el'))]
rows = []
for name, cmds in FILES.items():
    f = root / 'gnu-lisp' / f'{name}.el'
    p = subprocess.run([binary, *pre, '--', str(f), *map(str, lp)], capture_output=True, text=True, timeout=120)
    rep = None
    for line in reversed(p.stdout.splitlines()):
        try: rep = json.loads(line); break
        except ValueError: pass
    row = {'file': f'lisp/{name}.el', 'commands': cmds, 'exit': p.returncode}
    if rep is None:
        row['error'] = (p.stderr or p.stdout)[-400:]
    else:
        steps = [s for s in rep.get('steps', []) if s.get('file') == str(f.resolve())]
        ok = [s for s in steps if s.get('status') == 'ok']
        bad = next((s for s in steps if s.get('status') != 'ok'), None)
        chain = rep.get('chain') or []
        row.update(ok=rep.get('ok'), forms_ok=len(ok),
                   chain=[(c.get('file', '').split('/gnu-lisp/')[-1], c.get('ok')) if isinstance(c, dict) else c for c in chain][:20],
                   first_failure=None if bad is None else {k: bad.get(k) for k in ('line', 'form', 'index', 'status', 'error') if k in bad},
                   error=(rep.get('error') or '')[:300], diagnostic_condition=rep.get('diagnostic_condition'))
    rows.append(row)
    print(json.dumps(row, ensure_ascii=False)[:900])
out.write_text(json.dumps(rows, indent=1, ensure_ascii=False))
