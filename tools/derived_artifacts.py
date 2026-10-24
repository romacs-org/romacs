#!/usr/bin/env python3
"""Validate and explicitly regenerate the complete derived JSON artifact set."""
import json
from pathlib import Path
import re
import subprocess
import sys

ARTIFACTS = (
    'DIVERGENCES.json',
    'PRIMITIVES.json',
    'crates/romacs-introspect-live/data/registry.json',
    'crates/romacs-introspect/data/registry.json',
)
GENERATORS = (
    (sys.executable, 'tools/divergence_index.py'),
    (sys.executable, 'tools/primitive_index.py'),
    ('cargo', 'run', '-q', '-p', 'romacs-introspect-live', '--', '--write'),
)
MARKER = re.compile(rb'(?m)^[ \t]*(?:<{7}|={7}|>{7}|\|{7})')


def validate(root):
    for name in ARTIFACTS:
        data = (root / name).read_bytes()
        if MARKER.search(data):
            raise ValueError(f'{name}: unresolved conflict marker')
        try:
            json.loads(data)
        except (ValueError, UnicodeError) as exc:
            raise ValueError(f'{name}: invalid JSON: {exc}') from exc


def regenerate(root):
    """Recover indexes and bootstrap the generator from a valid committed snapshot."""
    for name in ARTIFACTS[:2]:
        (root / name).unlink(missing_ok=True)
    embedded = root / ARTIFACTS[3]
    try:
        bootstrap = embedded.read_bytes()
        if MARKER.search(bootstrap):
            raise ValueError('conflicted snapshot')
        json.loads(bootstrap)
    except (OSError, ValueError, UnicodeError):
        bootstrap = subprocess.run(
            ['git', 'show', 'HEAD:' + ARTIFACTS[3]], cwd=root,
            check=True, stdout=subprocess.PIPE).stdout
        if MARKER.search(bootstrap):
            raise ValueError('committed bootstrap snapshot has conflict markers')
        json.loads(bootstrap)
        embedded.parent.mkdir(parents=True, exist_ok=True)
        embedded.write_bytes(bootstrap)
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
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    try:
        globals()[args.action](args.root.resolve())
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        print(f'derived artifacts: {exc}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
