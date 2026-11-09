#!/usr/bin/env python3
"""Smells census, with floors that only go down (SMELL-FLOORS.json).

One report of the smells the cleanup removes, and a ratchet on each, so a
removed smell cannot come back. The scope is the unsafe census's: every .rs
file in a Cargo package, credited to its nearest package
(tools/lib/unsafe_census.py, which also keeps the unsafe floors in
UNSAFE-FLOORS.json; `report` shows its kinds and SAFETY coverage here too).

Per crate:
  allows      lint names in `allow(...)` or `expect(...)` attributes that are
              dead_code, the unused group, or an unused_* lint (one per name)
  todos       TODO, FIXME, XXX and HACK in comments
  long_files  files over `long_file_lines` lines

Per lint, across the workspace (`lints`): every rustc and Clippy warning of
`cargo clippy --workspace --all-targets -- -W clippy::pedantic`, each
distinct diagnostic once, however many targets compile its code. Lints are
capped at warn, so a deny-by-default lint is counted rather than stopping the
count.

Layout (`layout`):
  root                    the tree's top-level entries
  members_outside_crates  workspace members not under crates/
  legacy_primitives       primitives lib.rs registers through its own enum,
                          the second route beside `defsubr!` (PRIMITIVES.json)
  primitives_outside_prims  `defsubr! { name = ... }` declarations outside
                          PRIMS (the harness's own tests and probes)

check fails when a count rises above its floor, and when an entry is missing
from or extra to a layout list. Crate smells and layout counts move rarely, so
a slack one fails too: the change that drops one records it with `lower`.
Lint counts move with nearly every commit, so a lint's floor is a ceiling: a
drop passes, and `lower` records drops at checkpoints. Failing on a slack lint
would put this file in every commit and every merge. `lower` only lowers floors
and drops listed entries that are gone. Raising a floor or adding an entry is
a hand edit, made deliberately in its own commit with its reason.

usage: smells.py check|report|lower [--root ROOT] [--clippy-json FILE]
       smells.py report --floors   (counts in the floors file's format)
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tomllib
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import unsafe_census  # noqa: E402

FLOORS = Path('SMELL-FLOORS.json')
CRATE_SMELLS = ('allows', 'todos', 'long_files')
COUNTED = ('legacy_primitives', 'primitives_outside_prims')
PRIMS = 'crates/romacs-vm/src/prims/'
LEGACY_REGISTRY = 'crates/romacs-vm/src/lib.rs'
ALLOW = re.compile(r'\b(?:allow|expect)\s*\(([^)]*)\)')
ALLOWED_LINT = re.compile(r'(?<![:\w])(?:dead_code|unused(?:_\w+)?)\b')
TODO = re.compile(r'\b(?:TODO|FIXME|XXX|HACK)\b')
DEFSUBR = re.compile(r'\bdefsubr\s*!\s*\{\s*name\s*=')
CLIPPY = ('cargo', 'clippy', '--locked', '--offline', '--workspace', '--all-targets',
          '--message-format=json', '--', '-W', 'clippy::pedantic', '--cap-lints', 'warn')


def file_smells(src, long_file_lines):
    """SRC's smells, and how many primitives it declares."""
    code, comments, _ = unsafe_census.lex(src)
    smells = Counter(allows=sum(len(ALLOWED_LINT.findall(m[1])) for m in ALLOW.finditer(code)),
                     todos=sum(len(TODO.findall(text)) for _, _, text in comments),
                     long_files=int(len(src.splitlines()) > long_file_lines))
    return smells, len(DEFSUBR.findall(code))


def crate_census(root, long_file_lines):
    """({crate: Counter of CRATE_SMELLS}, [(lines, path)] of long files,
    {path outside PRIMS: primitives it declares})."""
    counts, long, strays = {}, [], {}
    for crate, name, src in unsafe_census.rust_sources(root):
        smells, declares = file_smells(src, long_file_lines)
        counts.setdefault(crate, Counter()).update(smells)
        if smells['long_files']:
            long.append((len(src.splitlines()), name))
        if declares and not name.startswith(PRIMS):
            strays[name] = declares
    return counts, sorted(long, reverse=True), strays


def place(root, span):
    """SPAN's (file, line, column), the file relative to ROOT when inside it, so the
    aliases rustc prints for one file (`tests/../src/x.rs`, absolute and relative
    build scripts) are one place."""
    path = os.path.normpath(os.path.join(root, span['file_name']))
    inside = os.path.relpath(path, root)
    return (path if inside.startswith('..') else inside, span['line_start'], span['column_start'])


def call_site(span):
    """The outermost macro call SPAN expands from, or SPAN itself."""
    while span.get('expansion'):
        span = span['expansion']['span']
    return span


def lint_census(root, lines):
    """{crate: Counter of lint codes} from cargo's JSON message LINES. A warning is
    its lint, message and primary spans, each with the call site it was expanded
    from, so one warning compiled by two targets counts once and a warning inside
    a shared macro counts once per call. LINES must end in a successful build."""
    root = os.path.realpath(root)
    dirs = {Path(root, d).resolve(): n for d, n in unsafe_census.packages(root).items()}
    seen, counts, finished = set(), {}, False
    for line in lines:
        if not line.startswith('{'):
            continue
        message = json.loads(line)
        diagnostic = message.get('message') or {}
        finished = message.get('reason') == 'build-finished' and message.get('success') is True
        if message.get('reason') != 'compiler-message' or diagnostic.get('level') != 'warning' \
                or not diagnostic.get('code'):
            continue
        spans = tuple(sorted((place(root, s), place(root, call_site(s)))
                             for s in diagnostic['spans'] if s['is_primary']))
        key = (diagnostic['code']['code'], diagnostic['message'], spans)
        if key in seen:
            continue
        seen.add(key)
        crate = dirs.get(Path(message['manifest_path']).parent.resolve(), '?')
        counts.setdefault(crate, Counter())[key[0]] += 1
    if not finished:
        raise ValueError('the clippy messages do not end in a successful build; '
                         'a failed or cut-off build counts nothing')
    return counts


def run_clippy(root):
    """Cargo's JSON message lines for the workspace's warnings."""
    done = subprocess.run(CLIPPY, cwd=root, stdout=subprocess.PIPE, text=True)
    if done.returncode:
        raise ValueError(f'`{" ".join(CLIPPY)}` failed with {done.returncode}; a failed build counts nothing')
    return done.stdout.splitlines()


def layout(root, strays):
    """{'root': [top-level entries], 'members_outside_crates': [...],
    'legacy_primitives': N, 'primitives_outside_prims': N}."""
    listed = subprocess.run(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'],
                            cwd=root, check=True, stdout=subprocess.PIPE).stdout.decode()
    top = sorted({name.split('/', 1)[0] for name in listed.split('\0') if name and (Path(root) / name).exists()})
    members = tomllib.loads((Path(root) / 'Cargo.toml').read_text())['workspace']['members']
    primitives = json.loads((Path(root) / 'PRIMITIVES.json').read_text())['primitives'].values()
    return {'root': top,
            'members_outside_crates': sorted(m for m in members if not m.startswith('crates/')),
            'legacy_primitives': sum(p['registered_at'].startswith(LEGACY_REGISTRY + ':') for p in primitives),
            'primitives_outside_prims': sum(strays.values())}


def measure(root, floors, clippy_json=None):
    """Everything check compares against FLOORS."""
    crates, long, strays = crate_census(root, floors['long_file_lines'])
    lines = Path(clippy_json).read_text().splitlines() if clippy_json else run_clippy(root)
    by_crate = lint_census(root, lines)
    return {'crates': crates, 'long': long, 'strays': strays, 'lints_by_crate': by_crate,
            'lints': sum(by_crate.values(), Counter()), 'layout': layout(root, strays)}


def bound(what, have, limit):
    """Why HAVE breaks the floor LIMIT for WHAT, or None."""
    if have > limit:
        return f'{what}: {have}, above its floor {limit}'
    if have < limit:
        return f'{what}: {have}, below its floor {limit}; record the drop with tools/lib/smells.py lower'
    return None


def listed(what, have, floor):
    found = [f'{what}: {entry} is new; adding one is a deliberate edit of {FLOORS}'
             for entry in sorted(set(have) - set(floor))]
    return found + [f'{what}: {entry} is gone; record it with tools/lib/smells.py lower'
                    for entry in sorted(set(floor) - set(have))]


def problems(floors, now):
    """Why NOW breaks FLOORS, one line each."""
    found = []
    for crate in sorted(set(floors['crates']) | set(now['crates'])):
        floor, have = floors['crates'].get(crate, {}), now['crates'].get(crate, Counter())
        found += [bound(f'{crate} {smell}', have[smell], floor.get(smell, 0)) for smell in CRATE_SMELLS]
    for lint in sorted(now['lints']):
        if now['lints'][lint] > floors['lints'].get(lint, 0):
            found.append(f'lint {lint}: {now["lints"][lint]}, above its ceiling {floors["lints"].get(lint, 0)}')
    for key in ('root', 'members_outside_crates'):
        found += listed(f'layout {key}', now['layout'][key], floors['layout'][key])
    found += [bound(f'layout {key}', now['layout'][key], floors['layout'][key]) for key in COUNTED]
    return [p for p in found if p]


def lowered(floors, now):
    """FLOORS lowered to NOW, never raised; zeros and gone entries dropped."""
    crates = {}
    for crate, floor in floors['crates'].items():
        new = {s: min(floor.get(s, 0), now['crates'].get(crate, Counter())[s]) for s in CRATE_SMELLS}
        if any(new.values()):
            crates[crate] = new
    lints = {lint: min(n, now['lints'][lint]) for lint, n in floors['lints'].items() if now['lints'][lint]}
    old = floors['layout']
    return {**floors, 'crates': dict(sorted(crates.items())), 'lints': dict(sorted(lints.items())),
            'layout': {'root': [e for e in old['root'] if e in now['layout']['root']],
                       'members_outside_crates': [m for m in old['members_outside_crates']
                                                  if m in now['layout']['members_outside_crates']],
                       **{key: min(old[key], now['layout'][key]) for key in COUNTED}}}


def as_floors(rule, long_file_lines, now):
    """NOW in the floors file's shape: what `report --floors` prints."""
    crates = {c: {s: t[s] for s in CRATE_SMELLS} for c, t in sorted(now['crates'].items())
              if any(t[s] for s in CRATE_SMELLS)}
    return {'rule': rule, 'long_file_lines': long_file_lines, 'crates': crates,
            'lints': dict(sorted(now['lints'].items())), 'layout': now['layout']}


def dump(floors):
    """FLOORS as SMELL-FLOORS.json text, one line per crate, lint and entry,
    so lanes that lower different ones merge cleanly."""
    def lines(items):
        return ',\n'.join(f'    {item}' for item in items)
    lay = floors['layout']
    return (f'{{\n  "rule": {json.dumps(floors["rule"])},\n'
            f'  "long_file_lines": {floors["long_file_lines"]},\n'
            f'  "crates": {{\n{lines(f"{json.dumps(c)}: {json.dumps(v)}" for c, v in floors["crates"].items())}\n  }},\n'
            f'  "lints": {{\n{lines(f"{json.dumps(k)}: {v}" for k, v in floors["lints"].items())}\n  }},\n'
            f'  "layout": {{\n    "root": [\n{lines("  " + json.dumps(e) for e in lay["root"])}\n    ],\n'
            f'    "members_outside_crates": {json.dumps(lay["members_outside_crates"])},\n'
            + ''.join(f'    "{key}": {lay[key]},\n' for key in COUNTED).rstrip(',\n') + '\n  }\n}\n')


def report(root, floors, now, top=12):
    unsafe = unsafe_census.report(unsafe_census.census(root))
    crates = [f'{"crate":32} {"allows":>7} {"todos":>6} {"long":>5}']
    total = Counter()
    for crate, tally in sorted(now['crates'].items(), key=lambda item: -sum(item[1].values())):
        if any(tally[s] for s in CRATE_SMELLS):
            total.update(tally)
            crates.append(f'{crate:32} {tally["allows"]:>7} {tally["todos"]:>6} {tally["long_files"]:>5}')
    crates.append(f'{"total":32} {total["allows"]:>7} {total["todos"]:>6} {total["long_files"]:>5}')
    long = [f'{lines:>7} {path}' for lines, path in now['long'][:top]]
    lints = [f'{now["lints"][lint]:>7} {floors["lints"].get(lint, 0):>7} {lint}'
             for lint in sorted(set(floors['lints']) | set(now['lints']),
                                key=lambda lint: (-now['lints'][lint], lint))]
    slack = sum(n - now['lints'][lint] for lint, n in floors['lints'].items() if n > now['lints'][lint])
    per_crate = [f'{sum(c.values()):>7} {crate}' for crate, c in
                 sorted(now['lints_by_crate'].items(), key=lambda item: -sum(item[1].values()))]
    lay = now['layout']
    return '\n'.join(['unsafe (tools/lib/unsafe_census.py):', unsafe, '', 'smells:', *crates, '',
                      f'longest files ({len(now["long"])} over the limit):', *long, '',
                      f'warnings by lint ({sum(now["lints"].values())}; count, ceiling; '
                      f'{slack} below the ceilings, for `lower`):', *lints, '',
                      'warnings by crate:', *per_crate, '',
                      f'layout: {len(lay["root"])} top-level entries, {len(lay["members_outside_crates"])} '
                      f'members outside crates/, {lay["legacy_primitives"]} primitives registered in lib.rs, '
                      f'{lay["primitives_outside_prims"]} declared outside {PRIMS}',
                      *(f'{n:>7} {path}' for path, n in sorted(now['strays'].items()))])


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=['check', 'report', 'lower'])
    parser.add_argument('--root', default='.')
    parser.add_argument('--clippy-json', help="cargo clippy's JSON messages, instead of running it")
    parser.add_argument('--floors', action='store_true', help='report: print the counts as a floors file')
    args = parser.parse_args()
    path = Path(args.root) / FLOORS
    floors = json.loads(path.read_text())
    try:
        now = measure(args.root, floors, args.clippy_json)
    except ValueError as exc:
        print(f'smells: {exc}', file=sys.stderr)
        return 1
    if args.command == 'report':
        sys.stdout.write(dump(as_floors(floors['rule'], floors['long_file_lines'], now)) if args.floors
                         else report(args.root, floors, now) + '\n')
        return 0
    if args.command == 'lower':
        path.write_text(dump(lowered(floors, now)))
        return 0
    found = problems(floors, now)
    for problem in found:
        print(f'smells: {problem}', file=sys.stderr)
    return 1 if found else 0


if __name__ == '__main__':
    sys.exit(main())
