#!/usr/bin/env python3
"""Validate and explicitly regenerate the derived files git still tracks.

Only the bundled autoloads remain. They leave git once Romacs's package code
makes them at build or install time; every other index is computed when asked
(tools/lib/primitive_index.py, tools/lib/divergence_index.py).
"""
import json
from pathlib import Path
import re
import subprocess
import sys

ARTIFACTS = (
    json.loads((Path(__file__).resolve().parents[2] / 'LISP-BUNDLED.json').read_text())['autoloads'],
)
GENERATORS = (
    (sys.executable, 'tools/lib/bundled_autoloads.py'),
)
MARKER = re.compile(rb'(?m)^[ \t]*(?:<{7}|={7}|>{7}|\|{7})')


def validate(root):
    for name in ARTIFACTS:
        data = (root / name).read_bytes()
        if MARKER.search(data):
            raise ValueError(f'{name}: unresolved conflict marker')
        stem = Path(name).name.encode()
        if not (data.startswith(b';;; ' + stem + b' --- ') and data.endswith(b'\n;;; ' + stem + b' ends here\n')):
            raise ValueError(f'{name}: not a complete generated Lisp file')


def regenerate(root):
    """Remake every artifact from its sources."""
    for command in GENERATORS:
        subprocess.run(command, cwd=root, check=True)
    validate(root)


def check(root):
    """Reject corruption before writing; compare exact input/output bytes."""
    validate(root)
    before = {name: (root / name).read_bytes() for name in ARTIFACTS}
    for command in GENERATORS:
        subprocess.run(command, cwd=root, check=True)
    validate(root)
    stale = [name for name in ARTIFACTS if (root / name).read_bytes() != before[name]]
    if stale:
        raise ValueError('stale derived artifacts regenerated; review and stage: ' + ', '.join(stale))


def main():
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('check', 'regenerate', 'validate'))
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    try:
        globals()[args.action](args.root.resolve())
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        print(f'derived artifacts: {exc}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
