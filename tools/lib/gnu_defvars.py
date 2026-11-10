#!/usr/bin/env python3
"""Record every variable GNU's C declares with DEFVAR at the Lisp pin.

`write` regenerates crates/romacs-vm/tests/fixtures/gnu_defvars.json from the
pinned commit's Git objects; `check` (run by tools/check-derived) fails when
the committed fixture differs. The GUI's DEFVAR census
(crates/romacs-vm-gui/src/defun_census.rs) checks that each is bound in the
loadup world or listed in DEFVAR-CENSUS.tsv.
"""
import argparse
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from gnu_arity import git, pin, sources  # noqa: E402
from lisp_oracle import GNU_SOURCE  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parents[2]
FIXTURE = 'crates/romacs-vm/tests/fixtures/gnu_defvars.json'
DEFVAR = re.compile(r'\bDEFVAR_(LISP|LISP_NOPRO|BOOL|INT|PER_BUFFER|KBOARD)\s*\(\s*"((?:[^"\\]|\\.)+)"')


def build(root, gnu):
    """{name: [file, ...]}: the src/*.c files declaring each, in order."""
    commit = pin(root)
    files = sources(gnu, commit)
    found = {}
    for file in sorted(f for f in files if f.endswith('.c')):
        for m in DEFVAR.finditer(files[file]):
            files_of = found.setdefault(m[2].replace('\\\\', '\\').replace('\\"', '"'), [])
            if file not in files_of:
                files_of.append(file)
    return {'provenance': {'generated_by': 'tools/lib/gnu_defvars.py', 'gnu_commit': commit,
                           'source': 'DEFVAR_LISP, DEFVAR_LISP_NOPRO, DEFVAR_BOOL, DEFVAR_INT, DEFVAR_PER_BUFFER '
                                     'and DEFVAR_KBOARD (NAME, ...) in src/*.c'},
            'defvars': dict(sorted(found.items()))}


def text(index):
    """One DEFVAR per line."""
    rows = ',\n'.join(f' {json.dumps(k)}: {json.dumps(v)}' for k, v in index['defvars'].items())
    return f'{{"provenance": {json.dumps(index["provenance"])},\n"defvars": {{\n{rows}\n}}}}\n'


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
        print(f'gnu_defvars: {FIXTURE} is stale; run tools/lib/gnu_defvars.py write', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
