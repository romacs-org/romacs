#!/usr/bin/env python3
"""The Lisp trees live under lisp/: GNU's in lisp/gnu, Romacs's in
lisp/romacs, side by side under the runtime root.

  python3 -B tools/lib/lisp_tree.py move [ROOT]      git mv gnu-lisp -> lisp/gnu, rlisp -> lisp/romacs,
                                                     and fixture trees so named alike
  python3 -B tools/lib/lisp_tree.py relink [ROOT]    rewrite the old names in every tracked file
  python3 -B tools/lib/lisp_tree.py check [ROOT]     exit 1 while an old tree or an old name is left

A reference is a path relative to the runtime root (the checkout, or an
installed share/romacs), so `gnu-lisp' becomes `lisp/gnu' wherever it names
the tree: in a path, a string, a comment.  Left alone: the pinned GNU trees
(lisp/gnu, gnu-tests/test), the vendored packages (lisp/romacs/deps), and
the `--gnu-lisp' options and identifiers that merely contain a name.

Deterministic and idempotent: each command run twice changes nothing more.
A branch that merges the move resolves its conflicts, then runs `move' (for
files it added under an old tree), then `relink', then `check'.

Transition code: it goes, with its test and its line in check-derived, once
every branch that predates the move has merged it and the smell floors no
longer list the old trees.
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

MOVES = (('gnu-lisp', 'lisp/gnu'), ('rlisp', 'lisp/romacs'))
NEW = dict(MOVES)

# GNU's pinned text and the vendored packages; this tool, its test and the
# layout test, which hold the old names as data; and the smell floors, which
# keep the old entries until the trunk lowers them at a checkpoint.
EXEMPT = ('lisp/gnu/', 'lisp/romacs/deps/', 'gnu-tests/test/', 'SMELL-FLOORS.json',
          'tools/lib/lisp_tree.py', 'tools/tests/test_lisp_tree.py', 'tools/tests/test_lisp_layout.py')

# An old name standing alone: not inside `--gnu-lisp' or `owned_in_rlisp',
# nor after a backslash, except a string's `\n' or `\t' (`\rlisp' is `\r'
# and `lisp').
OLD = re.compile(r'(?:(?<=\\[nt])|(?<![-\w\\]))(gnu-lisp|rlisp)(?![-\w])')


def git(root: Path, *args) -> str:
    return subprocess.run(['git', '-C', str(root), *args], check=True, capture_output=True,
                          text=True).stdout


def tracked(root: Path) -> list[str]:
    return sorted(p for p in git(root, 'ls-files', '-z').split('\0') if p)


def texts(root: Path) -> dict[str, str]:
    """{path: text} of the tracked text files a rewrite may touch."""
    tree = {}
    for path in tracked(root):
        full = root / path
        if path.startswith(EXEMPT) or not full.is_file() or full.is_symlink():
            continue
        data = full.read_bytes()
        if b'\0' in data[:8192]:
            continue
        try:
            tree[path] = data.decode('utf-8')
        except UnicodeDecodeError:
            continue
    return tree


def rewrite(text: str) -> str:
    return OLD.sub(lambda m: NEW[m[1]], text)


def plan(root: Path) -> dict[str, str]:
    """The rewritten text of each file that changes."""
    return {p: new for p, t in texts(root).items() if (new := rewrite(t)) != t}


def old_trees(paths: list[str]) -> list[str]:
    """Each directory named gnu-lisp or rlisp that holds a tracked file: the
    trees, and test fixtures that mirror the layout.  The outermost one of a
    path only."""
    trees = set()
    for path in paths:
        parts = path.split('/')
        old = next((i for i, part in enumerate(parts[:-1]) if part in NEW), None)
        if old is not None:
            trees.add('/'.join(parts[:old + 1]))
    return sorted(trees)


def moved(tree: str) -> str:
    """Where the old tree TREE goes: its name replaced by the new path."""
    head, _, name = tree.rpartition('/')
    return f'{head}/{NEW[name]}' if head else NEW[name]


def move(root: Path) -> list[str]:
    """Move each old tree, or what a branch added under it after the move."""
    if git(root, 'ls-files', '--unmerged'):
        raise SystemExit('unmerged paths: resolve the merge first (keep your name for a file you renamed)')
    done, paths = [], tracked(root)
    for old in old_trees(paths):
        new = moved(old)
        files = [p for p in paths if p.startswith(old + '/')]
        if not (root / new).exists():
            (root / new).parent.mkdir(parents=True, exist_ok=True)
            git(root, 'mv', old, new)
            done.append(f'{old} -> {new}')
            continue
        for path in files:
            target = new + path[len(old):]
            if (root / target).exists():
                raise SystemExit(f'{path}: {target} exists; merge them by hand')
            (root / target).parent.mkdir(parents=True, exist_ok=True)
            git(root, 'mv', path, target)
            done.append(f'{path} -> {target}')
        prune(root / old)
    return done


def prune(tree: Path) -> None:
    """Remove the directories a file-by-file move left empty."""
    for directory in sorted((p for p in tree.rglob('*') if p.is_dir()), key=lambda p: -len(p.parts)) + [tree]:
        if not any(directory.iterdir()):
            directory.rmdir()


def links(root: Path, paths: list[str]) -> list[str]:
    """Tracked symbolic links whose target names an old tree."""
    return [p for p in paths if not p.startswith(EXEMPT) and (root / p).is_symlink()
            and OLD.search(os.readlink(root / p))]


def leftovers(root: Path) -> list[str]:
    paths = tracked(root)
    stale = {*old_trees(paths), *(old for old, _ in MOVES if (root / old).exists())}
    problems = [f'{old}/ still exists' for old in sorted(stale)]
    problems += [f'{new}/ is missing' for _, new in MOVES if not (root / new).is_dir()]
    problems += [f'{p}: a link into an old tree; point it at the new one' for p in links(root, paths)]
    return problems + [f'{p}: names an old tree' for p in sorted(plan(root))]


def main(argv: list[str]) -> int:
    if not argv or argv[0] not in ('move', 'relink', 'check') or len(argv) > 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    root = Path(argv[1] if len(argv) > 1 else '.').resolve()
    if argv[0] == 'move':
        print('\n'.join(move(root)) or 'nothing to move')
    elif argv[0] == 'relink':
        changes = plan(root)
        for path, text in sorted(changes.items()):
            (root / path).write_bytes(text.encode('utf-8'))
            print(path)
        print(f'{len(changes)} files relinked', file=sys.stderr)
    else:
        problems = leftovers(root)
        print('\n'.join(problems) or 'lisp tree: ok')
        return 1 if problems else 0
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
