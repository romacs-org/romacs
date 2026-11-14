#!/usr/bin/env python3
"""The AGENTS.md files: the root's rules hold everywhere, a module's beside its code.

A file that names something gone misleads whoever reads it, so what a file
names in backticks must exist:

  a path from the repository root        `crates/romacs-vm/src/host.rs`, `tools/gate`
  a file by its name alone               `Cargo.lock`: some file of the tree has the name
  a make target                          `make gate-small`
  a test target of a package             `cargo test -p romacs-vm --test quit_async`
  a live harness                         `scaling_live`

A word with a slash is a path, but for a remote's branch (`origin/beta`). A
path that starts at a crate (`src/...`, `tests/...`) is refused: a reader does
not know which crate. A word in upper case that some AGENTS.md names alone in
backticks (`FILE`, `MODULE`) is a placeholder, and a path with one is held to
nothing. A module's file opens with `# ` and its directory, and is at most
LIMIT lines.

usage: agents_md.py check [--root ROOT]
       agents_md.py list [--root ROOT]
       agents_md.py notes [--root ROOT] [--trunk TRUNK] [PATH ...]
`list` prints the files: no list of them is kept anywhere. `notes` prints the
module files over the PATHs, or over what the branch changes since its
merge-base with TRUNK (default beta), nearest first.
"""
import argparse
import os
from pathlib import Path, PurePosixPath
import re
import shlex
import subprocess
import sys
from typing import NamedTuple

NAME = 'AGENTS.md'
LIMIT = 60
SPAN = re.compile(r'`([^`\n]+)`')
PATH = re.compile(r'[A-Za-z0-9_.+-]+(/[A-Za-z0-9_.+-]+)+/?$|[A-Za-z0-9_.+-]+/$')
FILE = re.compile(r'[A-Za-z0-9_+-]+(\.[A-Za-z0-9_+-]+)+$')
PLACEHOLDER = re.compile(r'[A-Z][A-Z0-9_]*$')
HARNESS = re.compile(r'[a-z0-9_]+_live$')
HARNESSES = 'crates/romacs-vm-gui/harness'
CRATE_RELATIVE = ('src', 'tests', 'harness')
# make's options that take the next word, and those that take it when it is a number.
MAKE_TAKES = ('-C', '-f', '-I', '-o', '-W', '--directory', '--file', '--makefile', '--include-dir')
MAKE_TAKES_NUMBER = ('-j', '-l', '--jobs', '--load-average', '--max-load')


class Tree(NamedTuple):
    """What the files are held to: the make TARGETS, the tracked files' NAMES and
    EXTENSIONS, and the PLACEHOLDERS the AGENTS.md files declare."""
    targets: list
    names: frozenset
    extensions: frozenset
    placeholders: frozenset


def spans(text):
    """The backtick spans of TEXT outside fenced blocks, each (LINE, SPAN)."""
    found, fenced = [], False
    for number, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith('```'):
            fenced = not fenced
        elif not fenced:
            found.extend((number, span) for span in SPAN.findall(line))
    return found


def words(span):
    """SPAN's words as a shell would take them, or split at blanks when it is no command line."""
    try:
        return shlex.split(span)
    except ValueError:
        return span.split()


def make_names(arguments):
    """The targets among a make command's ARGUMENTS: not its options, their values, or its variables."""
    found, skip = [], False
    for index, word in enumerate(arguments):
        if skip:
            skip = False
        elif word.startswith('-'):
            following = arguments[index + 1] if index + 1 < len(arguments) else ''
            skip = word in MAKE_TAKES or (word in MAKE_TAKES_NUMBER and following.replace('.', '').isdigit())
        elif '=' not in word:
            found.append(word)
    return found


def named(span):
    """What SPAN names that can be held to the tree: ('path', P), ('file', NAME),
    ('make', TARGET), ('test', PACKAGE, TARGET) and ('harness', NAME)."""
    split = words(span)
    found = [('path', word) for word in split if PATH.match(word)]
    # A number with a point is no file name.
    found.extend(('file', word) for word in split if FILE.match(word) and not word.replace('.', '').isdigit())
    if split[:1] == ['make']:
        found.extend(('make', word) for word in make_names(split[1:]))
    after = dict(zip(split, split[1:]))
    package, test = after.get('-p'), after.get('--test')
    if package and test:
        found.append(('test', package, test))
    found.extend(('harness', word) for word in split if HARNESS.match(word))
    return found


def make_targets(makefile):
    """The targets MAKEFILE's text defines, a pattern rule as a regular expression."""
    names = re.findall(r'^([A-Za-z0-9_.%-]+)\s*:', makefile, flags=re.M)
    return [re.compile(re.escape(name).replace('%', '.+') + '$') for name in names if name != '.PHONY']


def placeholder(word, placeholders):
    """Whether WORD, or a part of it between slashes, dots and dashes, is one of PLACEHOLDERS."""
    return any(part in placeholders for part in re.split(r'[/.-]', word))


def test_target(root, package, test):
    """Whether PACKAGE under ROOT has the test target TEST: a file or a directory
    with a main.rs under its tests/, or one its manifest declares."""
    crate = root / 'crates' / package
    if (crate / 'tests' / f'{test}.rs').is_file() or (crate / 'tests' / test / 'main.rs').is_file():
        return True
    manifest = crate / 'Cargo.toml'
    return manifest.is_file() and re.search(
        r'\[\[test\]\][^\[]*?\bname\s*=\s*"' + re.escape(test) + '"', manifest.read_text()) is not None


def missing(root, thing, tree):
    """Why THING, of `named`, is not in the tree at ROOT, or None."""
    kind = thing[0]
    if placeholder(' '.join(thing[1:]).replace(' ', '/'), tree.placeholders):
        return None
    if kind == 'path':
        path = thing[1]
        first = path.split('/')[0]
        if first == 'origin':
            return None
        if first in CRATE_RELATIVE:
            return 'starts at a crate; fix: write it from the repository root'
        if not (root / path).exists():
            return 'does not exist; fix: name what replaced it, or drop the line'
    elif kind == 'file':
        name = thing[1]
        known = any(f'.{part}' in tree.extensions for part in name.split('.')[1:])
        if known and name not in tree.names:
            return 'is the name of no file here; fix: name what replaced it, or drop the line'
    elif kind == 'make' and not any(target.match(thing[1]) for target in tree.targets):
        return f'is no target of the Makefile ({thing[1]}); fix: name the target that replaced it'
    elif kind == 'test' and not test_target(root, thing[1], thing[2]):
        return f'names no test target of {thing[1]} ({thing[2]}); fix: name the test file that holds it'
    elif kind == 'harness' and not (root / HARNESSES / f'{thing[1]}.py').is_file():
        return f'is no harness under {HARNESSES}; fix: name the harness that replaced it'
    return None


def problems(root, file, text, tree):
    """What is wrong with the AGENTS.md FILE (a path from ROOT) whose text is TEXT, held to TREE."""
    found = []
    lines = text.splitlines()
    module = PurePosixPath(file).parent.as_posix()
    if module != '.':
        if lines[:1] != [f'# {module}']:
            found.append(f"{file}:1: a module's file opens with `# {module}`; fix: make that its first line")
        if len(lines) > LIMIT:
            found.append(f'{file}:{LIMIT + 1}: {len(lines)} lines, over the {LIMIT} a module file may have; fix: keep what only this module needs')
    for number, span in spans(text):
        for why in dict.fromkeys(filter(None, (missing(root, thing, tree) for thing in named(span)))):
            found.append(f'{file}:{number}: `{span}` {why}')
    return found


def files(root):
    """Every AGENTS.md of the tree at ROOT git knows or would add, the root's first."""
    listed = subprocess.run(['git', '-C', str(root), 'ls-files', '-z', '--cached', '--others', '--exclude-standard',
                             '--', NAME, f'*/{NAME}'], check=True, stdout=subprocess.PIPE).stdout.decode()
    return sorted((name for name in listed.split('\0') if name and (root / name).is_file()), key=lambda name: (name.count('/'), name))


def tree(root, texts):
    """The `Tree` of the repository at ROOT, whose AGENTS.md files have the TEXTS."""
    tracked = subprocess.run(['git', '-C', str(root), 'ls-files', '-z', '--cached', '--others', '--exclude-standard'],
                             check=True, stdout=subprocess.PIPE).stdout.decode(errors='replace').split('\0')
    # A file deleted and not yet staged is still in the index, and is no file here.
    names = frozenset(PurePosixPath(name).name for name in tracked if name and (root / name).is_file())
    makefile = root / 'Makefile'
    return Tree(targets=make_targets(makefile.read_text()) if makefile.is_file() else [],
                names=names,
                extensions=frozenset(PurePosixPath(name).suffix for name in names) - {''},
                placeholders=frozenset(span for text in texts for _, span in spans(text) if PLACEHOLDER.match(span)))


def check(root):
    """What is wrong with the AGENTS.md files of the tree at ROOT."""
    texts = {file: (root / file).read_text() for file in files(root)}
    held = tree(root, texts.values())
    return [problem for file, text in texts.items() for problem in problems(root, file, text, held)]


def relative(root, path):
    """PATH, given from ROOT or absolute, as a path from ROOT with no `..`; None when it is outside."""
    whole = os.path.normpath(os.path.join(root, path))
    inside = os.path.relpath(whole, root)
    return None if inside.startswith('..') else PurePosixPath(inside).as_posix()


def over(paths, modules):
    """The module files among MODULES (paths of AGENTS.md files) that stand over
    one of PATHS, the deepest first. The root's file stands over everything and
    is not a module's."""
    found = []
    for path in paths:
        for parent in PurePosixPath(path).parents:
            file = (parent / NAME).as_posix()
            if parent.as_posix() != '.' and file in modules and file not in found:
                found.append(file)
    return sorted(found, key=lambda file: (-file.count('/'), file))


def changed(root, trunk):
    """The paths the tree at ROOT changes since its merge-base with TRUNK, uncommitted ones too."""
    def git(*args):
        return subprocess.run(['git', '-C', str(root), *args], check=True, stdout=subprocess.PIPE).stdout.decode()
    base = git('merge-base', 'HEAD', trunk).strip()
    return sorted(set(git('diff', '--name-only', '-z', base).split('\0')) | set(git('ls-files', '-z', '-mo', '--exclude-standard').split('\0')) - {''})


def notes(root, paths):
    """The module files over PATHS in the tree at ROOT, as text."""
    found = over(filter(None, (relative(root, path) for path in paths)), set(files(root)))
    if not found:
        return f'no module {NAME} stands over these paths; the root {NAME} holds'
    return '\n'.join(f'== {file}\n{(root / file).read_text().rstrip()}\n' for file in found).rstrip()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=('check', 'list', 'notes'))
    parser.add_argument('paths', nargs='*')
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument('--trunk', default='beta')
    args = parser.parse_args()
    if args.command == 'list':
        print('\n'.join(files(args.root)))
        return 0
    if args.command == 'notes':
        print(notes(args.root, args.paths or changed(args.root, args.trunk)))
        return 0
    found = check(args.root)
    for problem in found:
        print(problem, file=sys.stderr)
    return 1 if found else 0


if __name__ == '__main__':
    sys.exit(main())
