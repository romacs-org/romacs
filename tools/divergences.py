#!/usr/bin/env python3
"""The divergence ledger as one file per divergence, under ledger/divergences/.

`split` turns the old prose ledgers (DIVERGENCES.md and
crates/*/GNU-DIVERGENCES.md) into those files, losing nothing.
- A table row of DIVERGENCES.md becomes a file named by its ID, holding its
  section heading, its table's header and the row itself.
- A `## ID — title` section of a crate ledger becomes a file named by that ID,
  and is appended to the row of the same ID if there is one.
- An entry without an ID is named `U-` plus a slug of where it sits.
- Prose that belongs to no entry goes to README.md, in order.

`merge` is for a branch that still edits the old ledgers. Merge the trunk
with `git merge -X no-renames`, so git stops on each edited ledger instead
of guessing that a small one was renamed into some entry file, and run this
while it stops. It splits the branch's ledgers and
those of the merge base the same way, and applies only the branch's own
changes, entry by entry. An entry the trunk changed too is left with
conflict markers for a person to resolve. Then it removes the old ledgers.

usage: divergences.py split [--root ROOT]
       divergences.py merge [--root ROOT]
"""
import argparse
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import divergence_index  # noqa: E402

DIR = divergence_index.DIR
ROOT_LEDGER = 'DIVERGENCES.md'
CRATE_LEDGERS = 'crates/*/GNU-DIVERGENCES.md'
SEPARATOR = re.compile(r'^\|[\s:|-]+\|\s*$')
LINK = re.compile(r'^\[([^\]]+)\]\([^)]*\)$')
HEADING_ID = re.compile(r'^([A-Z][A-Z0-9]*(?:-[A-Z0-9]+)*)(?=\s*(?:—|/|$))')
AN_ID = re.compile(r'^[A-Za-z][A-Za-z0-9]*(?:-[A-Za-z0-9]+)*$')
README_HEAD = """# Deliberate divergences from GNU Emacs

One file per divergence, named by its ID. A file holds the section the
divergence belongs to, the table header and its row, and any longer
explanation below. A new divergence is a new file; `tools/divergence_index.py
SYMBOL` finds the ones that name a symbol. Files named `U-...` hold entries
that never had an ID. What follows is the ledgers' prose that belongs to no
single entry.
"""


def slug(text):
    return re.sub(r'[^a-z0-9]+', '-', text.lower()).strip('-')[:60]


def cells(line):
    return [c.strip() for c in re.sub(r'<!--.*?-->', '', line).strip().strip('|').split('|')]


def is_entry(line):
    """Whether divergence_index.parse reads LINE as a divergence row."""
    if re.search(r'<!--\s*not-a-divergence\s*-->', line):
        return False
    row = cells(line)
    return len(row) >= 4 and row[0].lower() not in divergence_index.NOT_AN_ID


def row_id(line):
    """The row's ID, if its first cell is one rather than a description."""
    ident = LINK.sub(r'\1', cells(line)[0])
    return ident if AN_ID.match(ident) else None


def unlinked(line):
    """LINE with a linked ID cell (`[ID](other-ledger#anchor)`) reduced to the ID."""
    return re.sub(r'^\| \[([^\]]+)\]\([^)]*\) \|', r'| \1 |', line)


def split_root(text):
    """({key: [section, header, separator, rows...]}, README lines)."""
    entries, readme = {}, []
    section, header, shown, lines = None, [], True, text.split('\n')
    for i, line in enumerate(lines):
        if re.match(r'^##+\s', line):
            section = line
            readme.append(line)
        elif line.startswith('| ') and i + 1 < len(lines) and SEPARATOR.match(lines[i + 1]):
            header, shown = [line, lines[i + 1]], False
        elif SEPARATOR.match(line):
            continue
        elif line.startswith('| ') and is_entry(line):
            ident = row_id(line)
            key = ident or 'U-' + slug(section or 'top') + '-' + slug(cells(line)[0] if cells(line)[0] not in ('—', '-', '') else cells(line)[1])
            entry = entries.setdefault(key, [section, *header])
            if unlinked(line) not in entry:
                entry.append(unlinked(line))
        elif not (i == 0 and line.startswith('# ')):
            if line.startswith('| ') and not shown:
                readme += header
                shown = True
            readme.append(line)
    return entries, readme


def split_crate(text, crate):
    """({key: section text}, preamble lines) for a crate ledger of `## ...` sections."""
    blocks, preamble, current = [], [], None
    for line in text.split('\n'):
        if line.startswith('## '):
            current = [line]
            blocks.append(current)
        elif current is None:
            if not line.startswith('# '):
                preamble.append(line)
        else:
            current.append(line)
    entries = {}
    for block in blocks:
        heading = block[0][3:].strip()
        found = HEADING_ID.match(heading)
        ids = {row_id(line) for line in block if line.startswith('| ') and is_entry(line)} - {None}
        key = found[1] if found else (ids.pop() if len(ids) == 1 else f'U-{crate}-{slug(heading)}')
        entries[key] = (entries[key] + '\n\n' if key in entries else '') + '\n'.join(block).strip('\n')
    return entries, preamble


def split(ledgers):
    """{file name: content} for the ledgers {path: text}, README.md included."""
    root_entries, readme = split_root(ledgers.get(ROOT_LEDGER, ''))
    files = {key: '\n'.join([lines[0] or '## (no section)', '', *lines[1:]]) for key, lines in root_entries.items()}
    for path in sorted(p for p in ledgers if p != ROOT_LEDGER):
        crate = path.split('/')[1]
        entries, preamble = split_crate(ledgers[path], crate)
        for key, text in entries.items():
            files[key] = files[key] + '\n\n' + text if key in files else text
        if any(line.strip() for line in preamble):
            readme += ['', f'## {crate}', *preamble]
    files = {f'{key}.md': text.strip('\n') + '\n' for key, text in files.items()}
    files['README.md'] = README_HEAD + '\n' + re.sub(r'\n{3,}', '\n\n', '\n'.join(readme)).strip('\n') + '\n'
    return files


def old_ledgers(read, paths):
    """{path: text} of the old ledgers among PATHS, through READ (None if absent)."""
    return {path: text for path in paths if (text := read(path)) is not None}


def git(root, *args, check=True):
    return subprocess.run(['git', '-C', str(root), *args], check=check, capture_output=True, text=True)


def show(root, rev):
    def read(path):
        done = git(root, 'show', f'{rev}:{path}', check=False)
        return done.stdout if done.returncode == 0 else None
    return read


def ledger_paths(root, *revs):
    paths = {ROOT_LEDGER}
    for rev in revs:
        listed = git(root, 'ls-tree', '-r', '--name-only', rev, '--', 'crates').stdout.split()
        paths |= {p for p in listed if p.endswith('/GNU-DIVERGENCES.md')}
    return sorted(paths)


def write(root, files):
    out = Path(root) / DIR
    out.mkdir(parents=True, exist_ok=True)
    for name, text in files.items():
        (out / name).write_text(text)


def cmd_split(root):
    paths = [ROOT_LEDGER] + sorted(p.relative_to(root).as_posix() for p in Path(root).glob(CRATE_LEDGERS))
    ledgers = old_ledgers(lambda p: (Path(root) / p).read_text() if (Path(root) / p).exists() else None, paths)
    write(root, split(ledgers))
    for path in ledgers:
        (Path(root) / path).unlink()
    print(f'divergences: {len(ledgers)} ledgers split into {DIR}/')
    return 0


def cmd_merge(root):
    """Apply the branch's own ledger edits since the merge base to the files."""
    head, other = 'HEAD', 'MERGE_HEAD'
    if git(root, 'rev-parse', '--verify', '-q', other, check=False).returncode:
        print('divergences: no merge in progress; run `git merge -X no-renames <trunk>` first', file=sys.stderr)
        return 2
    base = git(root, 'merge-base', head, other).stdout.strip()
    paths = ledger_paths(root, base, head)
    before = split(old_ledgers(show(root, base), paths))
    after = split(old_ledgers(show(root, head), paths))
    conflicts = []
    for name in sorted(set(before) | set(after)):
        was, now = before.get(name), after.get(name)
        if was == now:
            continue
        path = Path(root) / DIR / name
        trunk = path.read_text() if path.exists() else None
        if trunk == was or trunk == now:
            if now is None:
                path.unlink(missing_ok=True)
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(now)
            continue
        conflicts.append(name)
        path.write_text(f'<<<<<<< branch\n{now or ""}=======\n{trunk or ""}>>>>>>> trunk\n')
    git(root, 'rm', '-q', '--cached', '--ignore-unmatch', '--', *paths)
    for path in paths:
        (Path(root) / path).unlink(missing_ok=True)
    git(root, 'add', '-A', '--', str(DIR))
    if conflicts:
        print('divergences: resolve these entries, then git add them:', *conflicts, sep='\n  ', file=sys.stderr)
        return 1
    print(f'divergences: applied the branch\'s ledger edits to {DIR}/')
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=['split', 'merge'])
    parser.add_argument('--root', default='.')
    args = parser.parse_args()
    root = Path(args.root).resolve()
    return cmd_split(root) if args.command == 'split' else cmd_merge(root)


if __name__ == '__main__':
    sys.exit(main())
