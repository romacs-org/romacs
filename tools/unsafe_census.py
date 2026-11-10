#!/usr/bin/env python3
"""Unsafe Rust census, with per-crate ratchets (UNSAFE-FLOORS.json).

Scope: every .rs file Git tracks or would track (untracked files that are not
ignored count too) inside a Cargo package directory, credited to the nearest
enclosing package. Rust files outside any package (editor test fixtures) are
not compiled and are not counted.

What counts as a site. The file is lexed so that comments, string, byte,
raw and character literals are never mistaken for code. Each `unsafe` keyword
left in code is one site, of one kind:

  block   `unsafe {`
  fn      `unsafe fn NAME` and `unsafe extern "ABI" fn NAME`, declarations
          inside extern blocks included; `unsafe fn(...)` pointer types are
          types, not sites
  impl    `unsafe impl`
  trait   `unsafe trait` and `unsafe auto trait`
  extern  `unsafe extern "ABI" {` (edition 2024 requires the keyword)
  attr    `#[unsafe(no_mangle)]` and the other unsafe attributes
  other   any other use (macro patterns), counted so nothing hides

What counts as documented. A site is documented when a comment containing
`SAFETY:` (any case, as Clippy's undocumented_unsafe_blocks reads it) is
either earlier on the site's own line, or in the run of comment lines directly
above it. The run is the comment-only lines walking up from the site's line;
attribute lines (`#[...]`, one line each) are stepped over, and a blank line or
any other code ends it. For fn, impl and trait a `# Safety` heading in the
doc comment counts too. A trailing comment after code, or a SAFETY comment
above the start of a multi-line statement whose `unsafe` begins a later
line, does not count: put the comment on the line above the keyword.

The ratchet. UNSAFE-FLOORS.json records, per crate, `sites` and
`undocumented`. `check` fails when a crate's count rises above its floor, and
also when a floor is slack (the crate now has fewer), so a deletion is
recorded in the same change: run `lower`, which only ever lowers floors and
drops crates that reach zero. A crate absent from the file has floors of
zero. Raising a floor is a hand edit, made deliberately in its own commit.
Undocumented sites carry the coverage rule: a new unsafe site without a
SAFETY comment raises `undocumented` and fails, and deleting a documented
site never does.

The file also lists the crates where `unsafe_op_in_unsafe_fn` is not yet
denied (`unsafe_op_in_unsafe_fn_pending`, crate: reason). `check` fails when a
crate neither denies the lint (through `[lints] workspace = true`, its own
`unsafe_op_in_unsafe_fn = "deny"`, or `unsafe_code = "forbid"`) nor is
listed, and when a listed crate already denies it.

usage: unsafe_census.py check|report|lower [--root ROOT]
"""
import argparse
import bisect
import json
import re
import subprocess
import sys
import tomllib
from collections import Counter
from pathlib import Path

FLOORS = Path('UNSAFE-FLOORS.json')
KINDS = ('block', 'fn', 'impl', 'trait', 'extern', 'attr', 'other')
PENDING = 'unsafe_op_in_unsafe_fn_pending'

# The next lexically interesting thing: a comment, a literal, or `unsafe`.
TOKEN = re.compile(r'''
    (?P<line>//[^\n]*)
  | (?P<block>/\*)
  | (?P<raw>\b(?:br|cr|r)(?P<hashes>\#*)")
  | (?P<str>(?:\b[bc])?")
  | (?P<char>(?:\bb)?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]*\}|.)|[^\\'\n])')
  | (?P<unsafe>(?<!r\#)\bunsafe\b)
''', re.VERBOSE)
STRING_BODY = re.compile(r'(?:[^"\\]|\\.)*"', re.DOTALL)
BLOCK_EDGE = re.compile(r'/\*|\*/')
KIND = (
    ('block', re.compile(r'unsafe\s*\{')),
    ('fn', re.compile(r'unsafe\s+(?:extern\s+)?fn\s+[A-Za-z_$]')),
    ('impl', re.compile(r'unsafe\s+impl\b')),
    ('trait', re.compile(r'unsafe\s+(?:auto\s+)?trait\b')),
    ('extern', re.compile(r'unsafe\s+extern\s*\{')),
    ('attr', re.compile(r'unsafe\s*\(')),
)
SAFETY = re.compile(r'SAFETY:', re.IGNORECASE)
SAFETY_HEADING = re.compile(r'#\s*Safety\b')
# The lint, or the one group that contains it.
LINT_ALLOW = re.compile(r'\b(?:allow|expect|warn)\s*\([^)]*\b(?:unsafe_op_in_unsafe_fn|rust_2024_compatibility)\b')


def block_end(src, start):
    """End offset of the nested block comment opened at START."""
    depth, pos = 0, start
    for edge in BLOCK_EDGE.finditer(src, start):
        depth += 1 if edge.group() == '/*' else -1
        pos = edge.end()
        if depth == 0:
            return pos
    return len(src)


def lex(src):
    """(code, comments, keywords): SRC with comments and literals blanked,
    the comments as (start, end, text), and the offsets of `unsafe`."""
    blanks, comments, keywords = [], [], []
    pos = 0
    while (m := TOKEN.search(src, pos)):
        start = m.start()
        if m['unsafe']:
            keywords.append(start)
            pos = m.end()
            continue
        if m['line']:
            end = m.end()
            comments.append((start, end, m['line']))
        elif m['block']:
            end = block_end(src, start)
            comments.append((start, end, src[start:end]))
        elif m['raw']:
            close = '"' + m['hashes']
            found = src.find(close, m.end())
            end = len(src) if found < 0 else found + len(close)
        elif m['str']:
            body = STRING_BODY.match(src, m.end())
            end = body.end() if body else len(src)
        else:
            end = m.end()
        blanks.append((start, end))
        pos = end
    code = list(src)
    for start, end in blanks:
        for i in range(start, end):
            if code[i] != '\n':
                code[i] = ' '
    return ''.join(code), comments, keywords


def kind_of(code, offset):
    window = code[offset:offset + 200]
    return next((name for name, pattern in KIND if pattern.match(window)), 'other')


def is_site(code, offset, kind):
    """False for `unsafe fn(...)` pointer types, the one keyword use that is
    not an obligation of the code it appears in."""
    return kind != 'other' or not re.match(r'unsafe\s+(?:extern\s+)?fn\s*[(<]', code[offset:offset + 200])


def documented(kind, line, column, lines, comments_by_line):
    """Whether the site at LINE, COLUMN (0-based) carries a SAFETY comment
    under the module's counting rule."""
    heading = kind in ('fn', 'impl', 'trait')
    text = [t for start_col, t in comments_by_line.get(line, ()) if start_col < column]
    row = line - 1
    while row >= 0:
        stripped = lines[row].strip()
        if not stripped and row in comments_by_line:
            text.extend(t for _, t in comments_by_line[row])
        elif stripped.startswith('#') and stripped.endswith(']'):
            pass
        else:
            break
        row -= 1
    joined = '\n'.join(text)
    return bool(SAFETY.search(joined) or (heading and SAFETY_HEADING.search(joined)))


def file_census(src):
    """([(kind, documented, line)] for each unsafe site in SRC, the number
    of attributes in SRC that allow unsafe_op_in_unsafe_fn)."""
    code, comments, keywords = lex(src)
    newlines = [i for i, c in enumerate(src) if c == '\n']
    line_of = lambda offset: bisect.bisect_left(newlines, offset)  # noqa: E731
    line_start = lambda line: newlines[line - 1] + 1 if line else 0  # noqa: E731
    comments_by_line = {}
    for start, end, text in comments:
        first, last = line_of(start), line_of(max(start, end - 1))
        for row in range(first, last + 1):
            comments_by_line.setdefault(row, []).append((start - line_start(first) if row == first else 0, text))
    lines = code.split('\n')
    sites = []
    for offset in keywords:
        kind = kind_of(code, offset)
        if not is_site(code, offset, kind):
            continue
        line = line_of(offset)
        sites.append((kind, documented(kind, line, offset - line_start(line), lines, comments_by_line), line + 1))
    return sites, len(LINT_ALLOW.findall(code))


def packages(root):
    """{package directory relative to ROOT: package name}."""
    listed = subprocess.run(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z',
                             '--', 'Cargo.toml', '*/Cargo.toml'],
                            cwd=root, check=True, stdout=subprocess.PIPE).stdout.decode()
    found = {}
    for name in filter(None, listed.split('\0')):
        path = Path(root) / name
        if not path.is_file():
            continue
        manifest = tomllib.loads(path.read_text())
        if 'package' in manifest:
            found[Path(name).parent.as_posix()] = manifest['package']['name']
    return found


def owner(path, dirs):
    """The package directory enclosing PATH, or None."""
    parts = Path(path).parts
    for n in range(len(parts) - 1, -1, -1):
        candidate = Path(*parts[:n]).as_posix() if n else '.'
        if candidate in dirs:
            return candidate
    return None


def rust_sources(root):
    """(crate, path relative to ROOT, source text) for each .rs file in scope."""
    dirs = packages(root)
    listed = subprocess.run(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z', '--', '*.rs'],
                            cwd=root, check=True, stdout=subprocess.PIPE).stdout.decode()
    for name in sorted(set(filter(None, listed.split('\0')))):
        path = Path(root) / name
        package = owner(name, dirs)
        if package is not None and path.is_file():
            yield dirs[package], name, path.read_text(errors='surrogateescape')


def census(root):
    """{crate: Counter of kinds plus 'sites', 'undocumented', and 'allows'
    (source attributes that relax unsafe_op_in_unsafe_fn)}."""
    counts = {}
    for crate, _, src in rust_sources(root):
        tally = counts.setdefault(crate, Counter())
        sites, allows = file_census(src)
        tally['allows'] += allows
        for kind, ok, _ in sites:
            tally[kind] += 1
            tally['sites'] += 1
            tally['undocumented'] += not ok
    return counts


def level(rust, name):
    """A `[lints.rust]` entry's level: `name = "deny"` or `{ level = "deny", ... }`."""
    value = rust.get(name)
    return value.get('level') if isinstance(value, dict) else value


def lint_denied(manifest, workspace):
    """Whether MANIFEST's lints deny unsafe_op_in_unsafe_fn, or forbid unsafe."""
    lints = manifest.get('lints', {})
    rust = workspace.get('rust', {}) if lints.get('workspace') else lints.get('rust', {})
    return level(rust, 'unsafe_op_in_unsafe_fn') in ('deny', 'forbid') or level(rust, 'unsafe_code') == 'forbid'


def lint_problems(root, pending, counts):
    """Packages whose unsafe_op_in_unsafe_fn state disagrees with PENDING. A
    package denies the lint when its manifest does and no source attribute
    (or its group rust_2024_compatibility) relaxes it again."""
    workspace = tomllib.loads((Path(root) / 'Cargo.toml').read_text()).get('workspace', {}).get('lints', {})
    found = []
    names = set()
    for directory, name in sorted(packages(root).items()):
        names.add(name)
        denied = (lint_denied(tomllib.loads((Path(root) / directory / 'Cargo.toml').read_text()), workspace)
                  and not counts.get(name, Counter())['allows'])
        if denied and name in pending:
            found.append(f'{name} denies unsafe_op_in_unsafe_fn; drop it from {PENDING} in {FLOORS}')
        elif not denied and name not in pending:
            found.append(f'{name} does not deny unsafe_op_in_unsafe_fn; deny it, or list it in {PENDING} with a reason')
        elif name in pending and not str(pending[name]).strip():
            found.append(f'{name} is listed in {PENDING} without a reason')
    found += [f'{PENDING} names {name}, which is no package' for name in sorted(set(pending) - names)]
    return found


def problems(root, floors=None, counts=None):
    """Why ROOT's unsafe census breaks UNSAFE-FLOORS.json, one line each."""
    floors = floors if floors is not None else json.loads((Path(root) / FLOORS).read_text())
    counts = counts if counts is not None else census(root)
    recorded = floors.get('crates', {})
    found = []
    for crate in sorted(set(recorded) | set(counts)):
        floor = recorded.get(crate, {})
        now = counts.get(crate, Counter())
        for key, what in (('sites', 'unsafe sites'), ('undocumented', 'unsafe sites without a SAFETY: comment')):
            have, limit = now[key], floor.get(key, 0)
            if have > limit:
                found.append(f'{crate}: {have} {what}, above its floor {limit}')
            elif have < limit:
                found.append(f'{crate}: {have} {what}, below its floor {limit}; '
                             'record the drop with tools/unsafe_census.py lower')
    return found + lint_problems(root, floors.get(PENDING, {}), counts)


def lowered(floors, counts):
    """FLOORS with every crate's floors lowered to COUNTS, never raised."""
    crates = {}
    for crate, floor in floors.get('crates', {}).items():
        now = counts.get(crate, Counter())
        new = {key: min(floor.get(key, 0), now[key]) for key in ('sites', 'undocumented')}
        if new['sites']:
            crates[crate] = new
    return {**floors, 'crates': dict(sorted(crates.items()))}


def dump(floors):
    """FLOORS as UNSAFE-FLOORS.json text: one line per crate, so lanes that
    lower different crates merge cleanly."""
    crates = ',\n'.join(f'    {json.dumps(crate)}: {json.dumps(floor)}' for crate, floor in floors['crates'].items())
    pending = ',\n'.join(f'    {json.dumps(crate)}: {json.dumps(reason)}'
                          for crate, reason in sorted(floors.get(PENDING, {}).items()))
    return (f'{{\n  "rule": {json.dumps(floors["rule"])},\n  "crates": {{\n{crates}\n  }},\n'
            f'  "{PENDING}": {{\n{pending}\n  }}\n}}\n')


def report(counts):
    header = f'{"crate":32} ' + ' '.join(f'{k:>6}' for k in KINDS) + f' {"sites":>6} {"undoc":>6} {"cover":>6}'
    rows = [header]
    total = Counter()
    for crate, tally in sorted(((c, t) for c, t in counts.items() if t['sites']), key=lambda item: -item[1]['sites']):
        total.update(tally)
        rows.append(row(crate, tally))
    rows.append(row('total', total))
    return '\n'.join(rows)


def row(name, tally):
    cover = 100 * (tally['sites'] - tally['undocumented']) / tally['sites'] if tally['sites'] else 100
    return (f'{name:32} ' + ' '.join(f'{tally[k]:>6}' for k in KINDS)
            + f' {tally["sites"]:>6} {tally["undocumented"]:>6} {cover:>5.1f}%')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=['check', 'report', 'lower'])
    parser.add_argument('--root', default='.')
    args = parser.parse_args()
    if args.command == 'report':
        print(report(census(args.root)))
        return 0
    path = Path(args.root) / FLOORS
    floors = json.loads(path.read_text())
    if args.command == 'lower':
        path.write_text(dump(lowered(floors, census(args.root))))
        return 0
    found = problems(args.root, floors)
    for problem in found:
        print(f'unsafe-census: {problem}', file=sys.stderr)
    return 1 if found else 0


if __name__ == '__main__':
    sys.exit(main())
