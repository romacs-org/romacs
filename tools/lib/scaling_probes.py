#!/usr/bin/env python3
"""The scaling probes, one file per probe.

crates/romacs-vm-gui/harness/scaling/probes.el keeps the harness: the
`scaling-probe' macro, the helpers probes call, the runner, and a loop that
loads every probes/NAME.el beside it. Each probe lives in its own file, so
lanes adding probes no longer collide in one file.

`split` makes that layout from a probes.el that still holds its probes.
`merge` brings in a branch that still adds probes to the old file; run it
after `git merge --no-commit -X no-renames <trunk>` (tools/lib/entry_merge.py).

usage: scaling_probes.py split|merge [--root ROOT]
"""
import argparse
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import entry_merge  # noqa: E402

DIR = Path('crates/romacs-vm-gui/harness/scaling')
FILE = 'probes.el'
PROBE = re.compile(r'\((?:scaling-probe|scaling--completion-probe)\s+([^\s()]+)')
LOADER = """;; Every probe lives in probes/NAME.el beside this file.
(dolist (file (directory-files
               (expand-file-name "probes" (file-name-directory (or load-file-name buffer-file-name)))
               t "\\\\.el\\\\'"))
  (load file nil t))

"""
PROVIDE = "(provide 'scaling-probes)"


MODIFIER = re.compile(r'\\(?:[ACHMSs]-|\^)')


def char_end(text, i):
    """The offset past the character literal at TEXT[I] == '?': its modifier
    prefixes (`\\C-`, `\\M-`, `\\^`, ...) and then one character, itself
    perhaps escaped (`?\\M-\\)` is a character, not a closing paren)."""
    i += 1
    while (found := MODIFIER.match(text, i)):
        i = found.end()
    return i + (2 if text[i:i + 1] == '\\' else 1)


def forms(text):
    """[(start, end)] offsets of TEXT's top-level forms, read past strings,
    comments and character literals. Unbalanced text is refused."""
    found, depth, start, i = [], 0, None, 0
    while i < len(text):
        c = text[i]
        if c == ';':
            i = text.find('\n', i)
            i = len(text) if i < 0 else i
            continue
        if c == '"':
            i += 1
            while i < len(text) and text[i] != '"':
                i += 2 if text[i] == '\\' else 1
        elif c == '?' and (i == 0 or text[i - 1] in ' \t\n([\'`,'):
            i = char_end(text, i)
            continue
        elif c in '([':
            if depth == 0:
                start = i
            depth += 1
        elif c in ')]':
            depth -= 1
            if depth < 0:
                raise ValueError(f'a closing paren with nothing open, at line {text.count(chr(10), 0, i) + 1}')
            if depth == 0 and start is not None:
                found.append((start, i + 1))
                start = None
        i += 1
    if depth:
        raise ValueError(f'a form opened at line {text.count(chr(10), 0, start or 0) + 1} never closes')
    return found


def split_text(text):
    """(harness text, {name: probe file text}) of an old probes.el."""
    harness, probes, cut = [], {}, 0
    for start, end in forms(text):
        found = PROBE.match(text, start)
        if not found or (start and text[start - 1] != '\n'):
            continue
        top = start
        while top:  # the comment block directly above the probe goes with it
            prev = text.rfind('\n', 0, top - 1) + 1
            if not text[prev:top].startswith(';'):
                break
            top = prev
        end = text.find('\n', end)  # with any comment after it on its last line
        end = len(text) if end < 0 else end
        stop = end + 1
        stop = stop + 1 if text[stop:stop + 1] == '\n' else stop
        harness.append(text[cut:top])
        name = found[1]
        probes[name] = (f';;; {name}.el --- scaling probe  -*- lexical-binding: t -*-\n\n'
                        + text[top:end].rstrip('\n') + '\n')
        cut = stop
    rest = text[cut:]
    if PROVIDE in rest and LOADER not in ''.join(harness) + rest:
        rest = rest.replace(PROVIDE, LOADER + PROVIDE)
    return ''.join(harness) + rest, probes


def split(files):
    """{path under DIR: text} for {old path: text}, through split_text."""
    harness, probes = split_text(files.get((DIR / FILE).as_posix(), ''))
    return {FILE: harness, **{f'probes/{name}.el': text for name, text in probes.items()}}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=['split', 'merge'])
    parser.add_argument('--root', default='.')
    args = parser.parse_args()
    root = Path(args.root).resolve()
    if args.command == 'merge':
        return entry_merge.merge(root, split, lambda *revs: [(DIR / FILE).as_posix()], DIR, 'scaling probes')
    path = root / DIR / FILE
    try:
        out = split({(DIR / FILE).as_posix(): path.read_text()})
    except ValueError as exc:
        print(f'scaling probes: {exc}; nothing written', file=sys.stderr)
        return 2
    for name, text in out.items():
        (root / DIR / name).parent.mkdir(parents=True, exist_ok=True)
        (root / DIR / name).write_text(text)
    print(f'scaling probes: {len(out) - 1} probes split into {DIR}/probes/')
    return 0


if __name__ == '__main__':
    sys.exit(main())
