#!/usr/bin/env python3
"""One Lisp tree under lisp/ (U1): lisp/gnu/X -> lisp/X, and lisp/romacs/ by
subsystem, as tools/lib/lisp_moves.tsv says.

  python3 -B tools/lib/lisp_tree.py move [ROOT]      git mv every file under lisp/gnu/ and lisp/romacs/
  python3 -B tools/lib/lisp_tree.py relink [ROOT]    rewrite the old paths in every tracked file
  python3 -B tools/lib/lisp_tree.py check [ROOT]     exit 1 while an old tree or an old path is left

A reference is a path relative to the runtime root (the checkout, or an
installed share/romacs): `lisp/gnu/subr.el' becomes `lisp/subr.el',
`lisp/romacs/romacs-agent.el' becomes `lisp/agents/romacs-agent.el', in a
path, a string or a comment.  A reference that names no file the map places
(`lisp/romacs' alone, a pattern) is listed by `relink' for a hand fix.  Left
alone: GNU's pinned test tree (test/gnu/test), the bundled packages' own text,
and this tool, its map and the tests that hold the old paths as data.  Every
other file under lisp/, GNU's included, is ours and is rewritten like any.

Deterministic and idempotent.  A branch that merges the move resolves its
conflicts, then runs `move' (for files it added under an old tree), `relink'
and `check'.  Transition code: it goes, with its map and test, once no open
branch predates the move.
"""
from __future__ import annotations

import fnmatch
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
MAP = HERE / 'lisp_moves.tsv'
OLD_TREES = ('lisp/gnu', 'lisp/romacs')
EXEMPT = ('test/gnu/test/', 'lisp/packages/', 'lisp/romacs/gnosis/', 'lisp/romacs/jabber/',
          'lisp/romacs/deps/', 'tools/lib/lisp_tree.py', 'tools/lib/lisp_moves.tsv',
          'tools/tests/test_lisp_tree.py', 'tools/tests/test_lisp_layout.py')
# A path into an old tree: not inside a longer name (`lisp/gnu-tests'), its
# segments plain file-name characters.
OLD = re.compile(r'(?<![\w-])lisp/(?:gnu|romacs)(?![\w-])(?:/[\w.+@-]+)*/?')


class Unplaced(Exception):
    pass


def load_map(text: str) -> list[tuple[str, str, str]]:
    rows = []
    for line in text.splitlines():
        if line.strip() and not line.startswith('#'):
            kind, old, new = line.split('\t')
            if kind not in ('file', 'dir', 'rule', 'gone'):
                raise ValueError(f'lisp_moves.tsv: unknown kind {kind}')
            rows.append((kind, old, new))
    return rows


def target(path: str, rows) -> str:
    """Where PATH, a file under an old tree, goes."""
    for kind, old, new in rows:
        if kind == 'file' and path == old:
            return new
    for kind, old, why in rows:
        if kind == 'gone' and path.startswith(old):
            raise Unplaced(f'{path}: {why}; merge that first')
    dirs = sorted((r for r in rows if r[0] == 'dir'), key=lambda r: -len(r[1]))
    for _, old, new in dirs:
        if path.startswith(old):
            return new + path[len(old):]
    head, _, name = path.rpartition('/')
    if head == 'lisp/romacs':
        for kind, pattern, new in rows:
            if kind == 'rule' and fnmatch.fnmatchcase(name, pattern):
                return new + name
    raise Unplaced(f'place {path}: add a rule to tools/lib/lisp_moves.tsv or git mv it')


def directory_target(path: str, rows) -> str | None:
    """Where a directory reference goes, when the map says: a dir row's own
    directory, or a directory under lisp/gnu/."""
    for kind, old, new in sorted((r for r in rows if r[0] == 'dir'), key=lambda r: -len(r[1])):
        if path + '/' == old:
            return new.rstrip('/')
        if (path + '/').startswith(old):
            return (new + path[len(old):]).rstrip('/')
    return None


def git(root: Path, *args) -> str:
    return subprocess.run(['git', '-C', str(root), *args], check=True, capture_output=True,
                          text=True).stdout


def tracked(root: Path) -> list[str]:
    return sorted(p for p in git(root, 'ls-files', '-z').split('\0') if p)


NESTED = re.compile(r'^(.*/)(lisp/(?:gnu|romacs)/.*)$')


def old_files(paths: list[str]) -> list[str]:
    """Files under an old tree: the runtime's, and test fixtures that mirror
    the layout (a `lisp/romacs/' or `lisp/gnu/' inside another directory)."""
    return [p for p in paths if p.startswith(tuple(t + '/' for t in OLD_TREES)) or NESTED.match(p)]


def placed(path: str, rows) -> str:
    """Where PATH goes: a fixture's own tree moves as the runtime's does."""
    nested = NESTED.match(path)
    if nested and not path.startswith(tuple(t + '/' for t in OLD_TREES)):
        return nested.group(1) + target(nested.group(2), rows)
    return target(path, rows)


def move(root: Path, rows) -> list[str]:
    if git(root, 'ls-files', '--unmerged'):
        raise SystemExit('unmerged paths: resolve the merge first (keep your name for a file you renamed)')
    paths = tracked(root)
    plan, problems = {}, []
    for path in old_files(paths):
        try:
            plan[path] = placed(path, rows)
        except Unplaced as error:
            problems.append(str(error))
    clashes = [f'{p}: {t} exists' for p, t in plan.items() if (root / t).exists()]
    if problems or clashes:
        raise SystemExit('\n'.join(problems + clashes))
    for path, new in plan.items():
        (root / new).parent.mkdir(parents=True, exist_ok=True)
        git(root, 'mv', path, new)
    for path in plan:
        prune(root, (root / path).parent)
    return [f'{p} -> {t}' for p, t in plan.items()]


def prune(root: Path, directory: Path) -> None:
    """Remove DIRECTORY and its parents below ROOT while the move left them empty."""
    while directory != root and directory.is_dir() and not any(directory.iterdir()):
        directory.rmdir()
        directory = directory.parent


def texts(root: Path) -> dict[str, str]:
    tree = {}
    for path in tracked(root):
        full = root / path
        if path.startswith(EXEMPT) or path.startswith(('lisp/gnu/',)) or not full.is_file() or full.is_symlink():
            continue
        data = full.read_bytes()
        if b'\0' in data[:8192]:
            continue
        try:
            tree[path] = data.decode('utf-8')
        except UnicodeDecodeError:
            continue
    return tree


def rewrite(text: str, rows) -> tuple[str, list[str]]:
    """TEXT with each old path replaced, and the references the map cannot place."""
    unplaced = []

    def one(match: re.Match) -> str:
        ref = match.group(0)
        for candidate in (ref, ref.rstrip('.,;:')):
            tail = ref[len(candidate):]
            bare = candidate.rstrip('/')
            slash = '/' if candidate.endswith('/') else ''
            if bare == 'lisp/gnu':
                return 'lisp' + slash + tail
            moved = directory_target(bare, rows)
            if moved is None:
                try:
                    moved = target(bare, rows)
                except Unplaced:
                    # A library's load name: `(load "lisp/romacs/foo")' is foo.el's.
                    if slash or '.' in bare.rsplit('/', 1)[-1]:
                        continue
                    try:
                        moved = target(bare + '.el', rows)[:-len('.el')]
                    except Unplaced:
                        continue
            return moved + slash + tail
        unplaced.append(ref)
        return ref

    return OLD.sub(one, text), unplaced


def plan(root: Path, rows) -> tuple[dict[str, str], dict[str, list[str]]]:
    changes, unplaced = {}, {}
    for path, text in texts(root).items():
        new, left = rewrite(text, rows)
        if new != text:
            changes[path] = new
        if left:
            unplaced[path] = left
    return changes, unplaced


def leftovers(root: Path, rows) -> list[str]:
    problems = [f'{t}/ still exists' for t in OLD_TREES if (root / t).exists()]
    problems += [f'{p}: still under an old tree' for p in old_files(tracked(root))]
    changes, unplaced = plan(root, rows)
    problems += [f'{p}: names an old path' for p in sorted(changes)]
    problems += [f'{p}: names {", ".join(sorted(set(refs)))}, which the map cannot place'
                 for p, refs in sorted(unplaced.items())]
    return problems


def main(argv: list[str]) -> int:
    if not argv or argv[0] not in ('move', 'relink', 'check') or len(argv) > 2:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    root = Path(argv[1] if len(argv) > 1 else '.').resolve()
    rows = load_map(MAP.read_text())
    if argv[0] == 'move':
        print('\n'.join(move(root, rows)) or 'nothing to move')
    elif argv[0] == 'relink':
        changes, unplaced = plan(root, rows)
        for path, text in sorted(changes.items()):
            (root / path).write_bytes(text.encode('utf-8'))
            print(path)
        for path, refs in sorted(unplaced.items()):
            print(f'hand fix: {path}: {", ".join(sorted(set(refs)))}', file=sys.stderr)
        print(f'{len(changes)} files relinked, {len(unplaced)} need a hand fix', file=sys.stderr)
    else:
        problems = leftovers(root, rows)
        print('\n'.join(problems) or 'lisp tree: ok')
        return 1 if problems else 0
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
