#!/usr/bin/env python3
"""Record the arity of every DEFUN in GNU's src/*.c at the Lisp pin.

`write` regenerates crates/romacs-vm/tests/fixtures/gnu_defun_arity.json from
the pinned commit's Git objects; `check` (run by tools/check-derived)
fails when the committed fixture differs. tests/gnu_arity_audit.rs compares
it with the arity of each function Romacs defines under a DEFUN's name.
"""
import argparse
import json
import pathlib
import re
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from lisp_oracle import GNU_SOURCE  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = 'crates/romacs-vm/tests/fixtures/gnu_defun_arity.json'
DEFUN = re.compile(r'^DEFUN\s*\(\s*"((?:[^"\\]|\\.)+)"\s*,\s*\w+\s*,\s*\w+\s*,\s*(\w+)\s*,\s*(\w+)\s*,', re.M)
# DEFUNs that GNU's preloaded Lisp replaces unconditionally, and where; GNU
# callers see that definition's arity.
PRELOADED = {'frame-windows-min-size': 'lisp/window.el'}
# Where one name has a DEFUN per platform, the X build's wins; the pinned
# GNU oracle and GUI are GNU/Linux builds.
PLATFORM = re.compile(r'^(w32|ns|haiku|android|msdos|dos|pgtk)')


def pin(root):
    return json.loads((root / 'LISP-PIN.json').read_text())['gnu_commit']


def git(gnu, *args):
    # GNU's C sources are not all UTF-8; DEFUN names are ASCII.
    return subprocess.run(['git', '-C', str(gnu), *args], check=True, capture_output=True).stdout.decode('latin-1')


def sources(gnu, commit):
    """{file name: text} for src/*.c and src/*.h at COMMIT."""
    names = [line.split('\t', 1)[1] for line in git(gnu, 'ls-tree', commit, '--', 'src/').splitlines()
             if line.endswith(('.c', '.h')) and ' blob ' in line]
    return {pathlib.PurePosixPath(n).name: git(gnu, 'show', f'{commit}:{n}') for n in names}


def enum_value(name, headers):
    """NAME's value where it is an implicitly numbered enum member."""
    for text in headers:
        for body in re.findall(r'\benum\s+\w+\s*\{([^}]*)\}', re.sub(r'/\*.*?\*/', '', text, flags=re.S)):
            members = [m.strip() for m in body.split(',')]
            if name in members and not any('=' in m for m in members[:members.index(name) + 1]):
                return members.index(name)
    raise ValueError(f'gnu_arity: cannot resolve DEFUN arity {name}')


def arities(files):
    """{name: [MIN, MAX]}, MAX a count, "many" or "unevalled"."""
    headers = [files[f] for f in sorted(files) if f.endswith('.h')]
    count = lambda v: v.lower() if v in ('MANY', 'UNEVALLED') else int(v) if v.isdigit() else enum_value(v, headers)
    found = {}
    for file in sorted((f for f in files if f.endswith('.c')), key=lambda f: (bool(PLATFORM.match(f)), f)):
        for m in DEFUN.finditer(files[file]):
            name = m[1].replace('\\\\', '\\').replace('\\"', '"')
            found.setdefault(name, [count(m[2]), count(m[3])])
    return dict(sorted(found.items()))


def lisp_arity(name, text):
    """The arity of the top-level (defun NAME ARGLIST ...) in TEXT."""
    m = re.search(r'^\(defun ' + re.escape(name) + r'\s+\(([^)]*)\)', text, re.M)
    if m is None:
        raise ValueError(f'gnu_arity: no top-level defun {name}')
    counts, kind = {'required': 0, '&optional': 0}, 'required'
    for arg in m[1].split():
        if arg == '&rest':
            return [counts['required'], 'many']
        if arg == '&optional':
            kind = arg
        else:
            counts[kind] += 1
    return [counts['required'], counts['required'] + counts['&optional']]


def build(root, gnu):
    commit = pin(root)
    found = arities(sources(gnu, commit))
    for name, file in PRELOADED.items():
        found[name] = lisp_arity(name, git(gnu, 'show', f'{commit}:{file}'))
    return {'provenance': {'generated_by': 'tools/lib/gnu_arity.py', 'gnu_commit': commit,
                           'source': 'DEFUN (NAME, F, S, MIN, MAX, ...) in src/*.c; '
                                     'the preloaded Lisp definition for ' + ', '.join(PRELOADED)},
            'arity': found}


def text(index):
    """One DEFUN per line."""
    rows = ',\n'.join(f' {json.dumps(k)}: {json.dumps(v)}' for k, v in index['arity'].items())
    return f'{{"provenance": {json.dumps(index["provenance"])},\n"arity": {{\n{rows}\n}}}}\n'


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('command', choices=['check', 'write'])
    ap.add_argument('--root', type=pathlib.Path, default=ROOT)
    ap.add_argument('--gnu', default=GNU_SOURCE)
    a = ap.parse_args()
    fixture = a.root / FIXTURE
    fresh = text(build(a.root, a.gnu))
    if a.command == 'write':
        fixture.write_text(fresh)
        print(f'wrote {FIXTURE}')
        return 0
    if not fixture.exists() or fixture.read_text() != fresh:
        print(f'gnu_arity: {FIXTURE} is stale; run tools/lib/gnu_arity.py write', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
