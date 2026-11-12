#!/usr/bin/env python3
"""Git's merge driver for the host boundary allow-list.

    host_allow.py merge BASE OURS THEIRS

The list (crates/romacs-vm/tests/host_boundary_allow.txt, written by
tests/host_boundary_lint.rs) is a set of entries, each `PATH ITEM API MARK
REASON` on a line of its own, keyed by its first four words. A merge of text
conflicts when two branches change neighbouring lines; a merge of entries
conflicts only when they change one entry differently. `tools/derived setup`
installs the driver; without it git merges the text.
"""
import sys
from pathlib import Path

KEY = 4


def entries(text):
    """{key: line} of the list TEXT; ValueError on a key listed twice."""
    out = {}
    for line in text.splitlines():
        if line.strip() and not line.startswith('#'):
            key = tuple(line.split()[:KEY])
            if key in out:
                raise ValueError(f'{" ".join(key)} is listed twice')
            out[key] = line
    return out


def merged(base, ours, theirs):
    """(text, conflicts): the three-way merge of three lists' texts, entry by
    entry, under OURS's comment lines and in the order the lint writes, and
    the keys the sides changed differently. Such an entry keeps each side's
    line it has, which the lint then reports (listed twice, or stale)."""
    b, o, t = entries(base), entries(ours), entries(theirs)
    out = [line for line in ours.splitlines() if line.startswith('#')]
    conflicts = []
    for key in sorted(set(b) | set(o) | set(t)):
        was, mine, yours = b.get(key), o.get(key), t.get(key)
        if mine != was and yours != was and mine != yours:
            conflicts.append(' '.join(key))
            out.extend(line for line in (mine, yours) if line is not None)
        elif (line := yours if mine == was else mine) is not None:
            out.append(line)
    return '\n'.join(out) + '\n', conflicts


def merge_files(base, ours, theirs):
    """Write the merge over OURS: 0, or 1 when the sides changed an entry
    differently, so git leaves the file unmerged with both its lines in it.
    1, leaving OURS, when a side lists a key twice."""
    try:
        text, conflicts = merged(*(Path(p).read_text() for p in (base, ours, theirs)))
    except (OSError, ValueError) as error:
        print(f'host_allow: {error}', file=sys.stderr)
        return 1
    Path(ours).write_text(text)
    for key in conflicts:
        print(f'host_allow: {key} changed differently on the two sides', file=sys.stderr)
    return 1 if conflicts else 0


def main(argv):
    if len(argv) != 5 or argv[1] != 'merge':
        print(__doc__, file=sys.stderr)
        return 2
    return merge_files(*argv[2:])


if __name__ == '__main__':
    sys.exit(main(sys.argv))
