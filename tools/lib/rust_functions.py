#!/usr/bin/env python3
"""The functions of the workspace's own Rust, how long each is, and two floors.

Scope: every tracked (or trackable) .rs file under crates/ but crates/vendor/,
less what only tests compile: files under a package's tests/, benches/ and examples/,
`#[cfg(test)]` items and `#[test]` functions.  A file is lexed as
tools/lib/unsafe_census.py lexes it, so comments and literals are never code.

A function is each `fn` item with a body; one inside another counts on its
own and within its parent, as Clippy's too_many_lines counts them.  Its
length is its code lines: the lines of its body holding code, its braces'
lines between its braces, blank and comment-only lines not, a line of a
string literal that spans lines included: what too_many_lines measures.  A deliberately long function is declared in the
comment lines directly above it (its attributes stepped over):
`// SMELL(long-function): reason`.

The floors, counted in the same non-test code:
  unwraps        `.unwrap()` and `.expect(` calls
  unsafe_undocumented  unsafe sites with no SAFETY comment
                 (tools/lib/unsafe_census.py says what documents one)

tools/lib/smells.py holds the long functions to the merge-base as it holds
the Lisp ones (tools/lib/lisp_functions.py: no new one over
LONG_FUNCTION_LINES, none grows) and the floors' totals to theirs.

usage: rust_functions.py report [--root ROOT] [--over N]
"""
import argparse
import bisect
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath

sys.path.insert(0, str(Path(__file__).resolve().parent))
import lisp_functions  # noqa: E402
import unsafe_census  # noqa: E402

LONG_FUNCTION_LINES = 50
KIND = 'long_rust_functions'
SCOPE = 'crates/'
TEST_DIRS = ('tests', 'benches', 'examples')
FN = re.compile(r'\bfn\s+(r#)?([A-Za-z_][A-Za-z0-9_]*)')
TEST_ATTR = re.compile(r'#\s*\[\s*(?:cfg\s*\(\s*test\s*\)|test|tokio::test)\s*\]')
TEST_MOD = re.compile(r'#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*((?:#\s*\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;')
PATH_ATTR = re.compile(r'#\s*\[\s*path\s*=\s*"([^"]+)"\s*\]')
UNWRAP = re.compile(r'\.\s*(?:unwrap\s*\(\s*\)|expect\s*\()')
MARK = re.compile(r'\bSMELL\(long-function\):[ \t]*\S')


def own(path):
    """Whether PATH, from the root, is the workspace's own non-test Rust."""
    p = PurePosixPath(path)
    if p.suffix != '.rs' or not path.startswith(SCOPE) or path.startswith(unsafe_census.VENDOR):
        return False
    return not any(part in TEST_DIRS for part in p.parts[2:-1])


def matching(code, open_at):
    """The offset just past the brace or bracket that closes CODE's at OPEN_AT."""
    pairs = {'{': '}', '(': ')', '[': ']'}
    stack = [pairs[code[open_at]]]
    i, n = open_at + 1, len(code)
    while i < n and stack:
        c = code[i]
        if c in pairs:
            stack.append(pairs[c])
        elif c in ')]}':
            if c == stack[-1]:
                stack.pop()
        i += 1
    return i


def body_start(code, at):
    """The offset of the `{` opening the body of the fn whose signature
    starts at AT, or None for one with no body (`;` first)."""
    depth, i, n = 0, at, len(code)
    while i < n:
        c = code[i]
        if c in '([':
            depth += 1
        elif c in ')]':
            depth -= 1
        elif depth == 0 and c == ';':
            return None
        elif depth == 0 and c == '{':
            return i
        i += 1
    return None


def test_spans(code):
    """The (start, end) spans of CODE's items only tests compile: each
    `#[cfg(test)]` or `#[test]` item, to its body's close or its `;`."""
    spans = []
    for m in TEST_ATTR.finditer(code):
        i = m.end()
        start = body_start(code, i)
        semi = code.find(';', i)
        if start is None or (0 <= semi < start):
            spans.append((m.start(), semi + 1 if semi >= 0 else len(code)))
        else:
            spans.append((m.start(), matching(code, start)))
    return spans


def inside(spans, offset):
    return any(start <= offset < end for start, end in spans)


def code_lines(code, line_starts, start, end):
    """How many lines of CODE, its comments blanked and its literals kept,
    from START to END hold code: Clippy's too_many_lines counts a line of a
    string that spans lines."""
    first = bisect.bisect_right(line_starts, start) - 1
    last = bisect.bisect_right(line_starts, end - 1) - 1
    count = 0
    for line in range(first, last + 1):
        lo = max(line_starts[line], start)
        hi = min(line_starts[line + 1] - 1 if line + 1 < len(line_starts) else len(code), end)
        if code[lo:hi].strip():
            count += 1
    return count


def without(src, comments):
    """SRC with its COMMENTS, (start, end, text), blanked, its newlines kept."""
    out, at = [], 0
    for start, end, _ in comments:
        out.append(src[at:start])
        out.append(re.sub(r'[^\n]', ' ', src[start:end]))
        at = end
    out.append(src[at:])
    return ''.join(out)


def test_modules(path, src):
    """The files the `#[cfg(test)] mod NAME;' declarations of SRC, at PATH,
    load: NAME.rs and NAME/mod.rs beside a mod.rs, lib.rs or main.rs, else
    in the directory named for PATH; or the file a `#[path]' names, beside
    PATH.  Read from SRC as written: a `#[path]' value is a literal."""
    p = PurePosixPath(path)
    home = p.parent if p.name in ('mod.rs', 'lib.rs', 'main.rs') else p.parent / p.stem
    found = set()
    for m in TEST_MOD.finditer(src):
        named = PATH_ATTR.search(m[1])
        found |= {str(p.parent / named[1])} if named else {str(home / f'{m[2]}.rs'), str(home / m[2] / 'mod.rs')}
    return found


def file_census(src):
    """(functions, unwraps, undocumented) of SRC: functions as
    [(name, line, lines, code, declared)], the counts outside test items."""
    code, comments, keywords = unsafe_census.lex(src)
    line_of, note = unsafe_census.annotator(src, code, comments)
    starts = [0] + [m.end() for m in re.finditer('\n', code)]
    uncommented = without(src, comments)
    tests = test_spans(code)
    found = []
    for m in FN.finditer(code):
        if inside(tests, m.start()):
            continue
        start = body_start(code, m.end())
        if start is None:
            continue
        end = matching(code, start)
        first, last = line_of(m.start()), line_of(end - 1)
        found.append((m.group(2), first + 1, last - first + 1,
                      code_lines(uncommented, starts, start, end) - 2 if last > first else 1,
                      bool(MARK.search(note(m.start())))))
    unwraps = sum(1 for m in UNWRAP.finditer(code) if not inside(tests, m.start()))
    undocumented = 0
    for offset in keywords:
        kind = unsafe_census.kind_of(code, offset)
        if unsafe_census.is_site(code, offset, kind) and not inside(tests, offset) \
                and not unsafe_census.documented(kind, note(offset)):
            undocumented += 1
    return found, unwraps, undocumented


def census(sources):
    """({key: (path, Function)}, {'unwraps': n, 'unsafe_undocumented': n}) of
    the own files among SOURCES, (path, text) pairs; keys as
    lisp_functions.census makes them, `PATH NAME' then `#K'."""
    found, floors = {}, {'unwraps': 0, 'unsafe_undocumented': 0}
    sources = [(path, text) for path, text in sorted(sources) if own(path)]
    tests = set().union(*(test_modules(path, text) for path, text in sources))
    for path, text in sources:
        if path in tests:
            continue
        functions, unwraps, undocumented = file_census(text)
        floors['unwraps'] += unwraps
        floors['unsafe_undocumented'] += undocumented
        for name, line, lines, code, declared in functions:
            key, k = f'{path} {name}', 1
            while key in found:
                k += 1
                key = f'{path} {name}#{k}'
            found[key] = (path, lisp_functions.Function(name, 'fn', line, lines, code, 0, declared))
    return found, floors


def working_sources(root):
    """(path, text) of the own Rust git has, or would have, in the working tree at ROOT."""
    names = lisp_functions.git(root, 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--',
                               SCOPE).split('\0')
    files = [(name, Path(root) / name) for name in names if name and own(name)]
    return [(name, lisp_functions.read_file(path)) for name, path in files if path.is_file()]


def commit_sources(root, commit):
    """(path, text) of the own Rust in COMMIT, read from git's objects."""
    return lisp_functions.blobs(root, commit, SCOPE, own)


def tree_census(root):
    return census(working_sources(root))


def commit_census(root, commit):
    return census(commit_sources(root, commit))


def long_functions(found, limit=LONG_FUNCTION_LINES):
    return lisp_functions.long_functions(found, limit)


def floor_rises(base, tree):
    """Why TREE's floors, {name: n}, rise above BASE's, one line each."""
    fixes = {'unwraps': 'handle the error, or return it with `?`',
             'unsafe_undocumented': 'put a `// SAFETY:` comment on the line above its `unsafe`'}
    return [f'{KIND}: {name}: {tree[name]} in non-test Rust, above the {base.get(name, 0)} of the merge-base; '
            f'fix: {fixes[name]}' for name in sorted(tree) if tree[name] > base.get(name, 0)]


def summary(found_floors):
    """What the ratchet compares of a census, (found, floors), as JSON keeps it:
    {'long': long_functions, 'keys': every function's key, 'floors': floors}."""
    found, floors = found_floors
    return {'long': long_functions(found), 'keys': sorted(found), 'floors': dict(floors)}


def rises(base, tree):
    """Why TREE adds to BASE, one line each: both summaries."""
    return lisp_functions.rises(base['long'], tree['long'], base['keys'], tree['keys'],
                                LONG_FUNCTION_LINES, KIND) + floor_rises(base['floors'], tree['floors'])


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=['report'])
    parser.add_argument('--root', default='.')
    parser.add_argument('--over', type=int, default=LONG_FUNCTION_LINES + 1)
    args = parser.parse_args()
    found, floors = tree_census(args.root)
    print(f'{"code":>5} {"lines":>5}   function  file:line')
    print(lisp_functions.report(found, args.over))
    print(f'{len(long_functions(found))} over {LONG_FUNCTION_LINES} code lines; {floors}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
