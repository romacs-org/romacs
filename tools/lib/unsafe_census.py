#!/usr/bin/env python3
"""Unsafe Rust census: what counts as a site, as documented, and as declared.

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

What annotates a site: the comments earlier on its own line, and the run of
comment lines directly above it. The run is the comment-only lines walking up
from the site's line; attribute lines (`#[...]`, one line each) are stepped
over, and a blank line or any other code ends it. A trailing comment after
code, or a comment above the start of a multi-line statement whose `unsafe`
begins a later line, does not annotate it: put the comment on the line above
the keyword.

A site is documented when its annotation holds `SAFETY:` (any case, as
Clippy's undocumented_unsafe_blocks reads it); for fn, impl and trait a
`# Safety` heading in the doc comment counts too. A site is declared when its
annotation also holds `UNSAFE(reason)`: why no crate in Cargo.lock covers the
call safely (AGENTS.md). tools/lib/smells.py holds the workspace's undeclared
and undocumented sites to the merge-base's: a new site is declared where it
occurs, and an undocumented one never comes back.

`check` holds every package to denying `unsafe_op_in_unsafe_fn` (through
`[lints] workspace = true`, its own `unsafe_op_in_unsafe_fn = "deny"`, or
`unsafe_code = "forbid"`), with no source attribute relaxing it again.

usage: unsafe_census.py check|report [--root ROOT]
"""
import argparse
import bisect
import re
import subprocess
import sys
import tomllib
from collections import Counter
from pathlib import Path

KINDS = ('block', 'fn', 'impl', 'trait', 'extern', 'attr', 'other')

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
DECLARED = re.compile(r'\bUNSAFE\(\s*[^)\s][^)]*\)')
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


def annotation(line, column, lines, comments_by_line):
    """The comment text annotating the code at LINE, COLUMN (0-based): the
    comments earlier on its line and the run of comment lines directly above."""
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
    return '\n'.join(text)


def documented(kind, note):
    """Whether NOTE, a site's annotation, documents a site of KIND."""
    return bool(SAFETY.search(note) or (kind in ('fn', 'impl', 'trait') and SAFETY_HEADING.search(note)))


def annotator(src, code, comments):
    """(line of an offset, the annotation of an offset) for SRC lexed into CODE and COMMENTS."""
    newlines = [i for i, c in enumerate(src) if c == '\n']
    line_of = lambda offset: bisect.bisect_left(newlines, offset)  # noqa: E731
    line_start = lambda line: newlines[line - 1] + 1 if line else 0  # noqa: E731
    comments_by_line = {}
    for start, end, text in comments:
        first, last = line_of(start), line_of(max(start, end - 1))
        for row in range(first, last + 1):
            comments_by_line.setdefault(row, []).append((start - line_start(first) if row == first else 0, text))
    lines = code.split('\n')

    def note(offset):
        line = line_of(offset)
        return annotation(line, offset - line_start(line), lines, comments_by_line)
    return line_of, note


def file_census(src):
    """([(kind, documented, declared, line)] for each unsafe site in SRC, the
    number of attributes in SRC that allow unsafe_op_in_unsafe_fn)."""
    code, comments, keywords = lex(src)
    line_of, note = annotator(src, code, comments)
    sites = []
    for offset in keywords:
        kind = kind_of(code, offset)
        if is_site(code, offset, kind):
            text = note(offset)
            sites.append((kind, documented(kind, text), bool(DECLARED.search(text)), line_of(offset) + 1))
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
    """{crate: Counter of kinds plus 'sites', 'undocumented', 'declared', and
    'allows' (source attributes that relax unsafe_op_in_unsafe_fn)}."""
    counts = {}
    for crate, _, src in rust_sources(root):
        tally = counts.setdefault(crate, Counter())
        sites, allows = file_census(src)
        tally['allows'] += allows
        for kind, ok, declared, _ in sites:
            tally[kind] += 1
            tally['sites'] += 1
            tally['undocumented'] += not ok
            tally['declared'] += declared
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


def lint_problems(root, counts):
    """Packages that do not deny unsafe_op_in_unsafe_fn. A package denies the
    lint when its manifest does and no source attribute (or its group
    rust_2024_compatibility) relaxes it again."""
    workspace = tomllib.loads((Path(root) / 'Cargo.toml').read_text()).get('workspace', {}).get('lints', {})
    return [f'{name} does not deny unsafe_op_in_unsafe_fn; deny it'
            for directory, name in sorted(packages(root).items())
            if not lint_denied(tomllib.loads((Path(root) / directory / 'Cargo.toml').read_text()), workspace)
            or counts.get(name, Counter())['allows']]


def report(counts):
    header = (f'{"crate":32} ' + ' '.join(f'{k:>6}' for k in KINDS)
              + f' {"sites":>6} {"undoc":>6} {"decl":>6} {"cover":>6}')
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
            + f' {tally["sites"]:>6} {tally["undocumented"]:>6} {tally["declared"]:>6} {cover:>5.1f}%')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=['check', 'report'])
    parser.add_argument('--root', default='.')
    args = parser.parse_args()
    counts = census(args.root)
    if args.command == 'report':
        print(report(counts))
        return 0
    found = lint_problems(args.root, counts)
    for problem in found:
        print(f'unsafe-census: {problem}', file=sys.stderr)
    return 1 if found else 0


if __name__ == '__main__':
    sys.exit(main())
