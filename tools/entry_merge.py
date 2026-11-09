#!/usr/bin/env python3
"""Merge a branch that still edits one big file into the trunk's files-per-entry.

When the trunk has split a file that every lane appends to into one file per
entry, a branch made before the split still edits the old file. Merge the
trunk with `git merge --no-commit -X no-renames <trunk>`:

- no-renames, or git may take a small old file as renamed into some entry
  file and apply the branch's edit there;
- --no-commit, so the merge stays open even where git merged the old file
  without complaint, for instance putting the branch's new entry into what
  the trunk left of that file.

Then this starts from the trunk's own version of every old file, splits the
merge base's and the branch's old files the way the trunk did, and applies
only the branch's changes, entry by entry. An entry the trunk changed too
gets conflict markers for a person to resolve.
"""
import subprocess
import sys
from pathlib import Path


def git(root, *args, check=True):
    return subprocess.run(['git', '-C', str(root), *args], check=check, capture_output=True, text=True)


def show(root, rev):
    """A reader of PATH at REV, None where it does not exist."""
    def read(path):
        done = git(root, 'show', f'{rev}:{path}', check=False)
        return done.stdout if done.returncode == 0 else None
    return read


def conflict(root, rel, was, now, trunk):
    """Leave REL unmerged: conflict markers in the tree and the three
    versions in the index, so nothing commits until a person resolves it."""
    path = Path(root) / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f'<<<<<<< branch\n{now or ""}=======\n{trunk or ""}>>>>>>> trunk\n')
    stages = []
    for stage, text in ((1, was), (2, now), (3, trunk)):
        if text is not None:
            blob = subprocess.run(['git', '-C', str(root), 'hash-object', '-w', '--stdin'], input=text,
                                  check=True, capture_output=True, text=True).stdout.strip()
            stages.append(f'100644 {blob} {stage}\t{rel}')
    git(root, 'update-index', '--force-remove', '--', rel)
    subprocess.run(['git', '-C', str(root), 'update-index', '--index-info'], input='\n'.join(stages) + '\n',
                   check=True, capture_output=True, text=True)


def merge(root, split, paths, out, label, unstable=lambda name: False):
    """Apply HEAD's changes since the merge base with MERGE_HEAD to OUT.

    SPLIT maps {old path: text} to {path under OUT: content}; PATHS(*revs)
    names the old files that may exist at those revisions. An entry whose name
    UNSTABLE says is derived from its content cannot be told apart from a
    rename, so the branch adding or deleting one is left for a person.
    Returns the exit status: 0 applied, 1 entries left in conflict, 2 no
    merge in progress."""
    if git(root, 'rev-parse', '--verify', '-q', 'MERGE_HEAD', check=False).returncode:
        print(f'{label}: no merge in progress; run `git merge --no-commit -X no-renames <trunk>` first',
              file=sys.stderr)
        return 2
    base = git(root, 'merge-base', 'HEAD', 'MERGE_HEAD').stdout.strip()
    old = paths(base, 'HEAD')

    def entries(rev):
        read = show(root, rev)
        return split({p: text for p in old if (text := read(p)) is not None})

    before, after = entries(base), entries('HEAD')
    for p in old:
        if git(root, 'cat-file', '-e', f'MERGE_HEAD:{p}', check=False).returncode == 0:
            git(root, 'checkout', 'MERGE_HEAD', '--', p)
        else:
            git(root, 'rm', '-q', '--cached', '--ignore-unmatch', '--', p)
            (Path(root) / p).unlink(missing_ok=True)
    conflicts = []
    for name in sorted(set(before) | set(after)):
        was, now = before.get(name), after.get(name)
        if was == now:
            continue
        path = Path(root) / out / name
        trunk = path.read_text() if path.exists() else None
        if trunk in (was, now) and not (unstable(name) and None in (was, now)):
            if now is None:
                path.unlink(missing_ok=True)
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(now)
            continue
        conflicts.append((name, was, now, trunk))
    git(root, 'add', '-A', '--', str(out))
    for name, was, now, trunk in conflicts:
        conflict(root, (Path(out) / name).as_posix(), was, now, trunk)
    conflicts = [name for name, *_ in conflicts]
    if conflicts:
        print(f'{label}: resolve these entries, then git add them:', *conflicts, sep='\n  ', file=sys.stderr)
        return 1
    print(f'{label}: applied the branch\'s edits to {out}/')
    return 0
