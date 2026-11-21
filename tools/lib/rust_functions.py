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
import functools
import posixpath
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
BRACKET = re.compile(r'[{}()\[\]]')
ATTR = re.compile(r'#\s*\[')
NEXT_ATTR = re.compile(r'\s*#\s*\[')
TEST_NAME = re.compile(r'\s*(?:test|tokio::test)\s*')
CFG = re.compile(r'\s*cfg\s*\((.*)\)\s*', re.DOTALL)
PATH_ATTR = re.compile(r'#\s*\[\s*path\s*=\s*"([^"]+)"\s*\]')
MOD_DECL = re.compile(r'\bmod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;')
CONTAINER = re.compile(r'\b(impl|trait|mod)\b')
# A field's or a parameter's head: what an attribute owns that is no item or
# statement (`#[cfg(test)] pub(crate) x: T`, `fn f(#[cfg(test)] x: u8)`, a
# struct literal's shorthand `#[cfg(test)] x,`).
FIELD = re.compile(r'\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:mut\s+)?[A-Za-z_][A-Za-z0-9_]*\s*(?::(?!:)|,|\})')
# Macros whose input is tokens, not items: a `fn` in them is no function here.
TOKEN_MACROS = re.compile(r'\b(?:stringify|concat|quote|quote_spanned)\s*!\s*[(\[{]')
QUALIFIERS = ('pub', 'async', 'const', 'unsafe', 'extern', 'default')
UNWRAP = re.compile(r'\.\s*(?:unwrap\s*\(\s*\)|expect\s*\()')
MARK = re.compile(r'\bSMELL\(long-function\):[ \t]*\S')


def own(path):
    """Whether PATH, from the root, is the workspace's own non-test Rust."""
    p = PurePosixPath(path)
    if p.suffix != '.rs' or not path.startswith(SCOPE) or path.startswith(unsafe_census.VENDOR):
        return False
    return not any(part in TEST_DIRS for part in p.parts[2:-1])


@functools.lru_cache(maxsize=4)
def pairs(code):
    """{offset of an opening bracket: offset just past the one closing it} of
    CODE, `{` `(` `[` matched in one pass; an unclosed one closes at the end."""
    closer = {'{': '}', '(': ')', '[': ']'}
    found, stack = {}, []
    for m in BRACKET.finditer(code):
        i, c = m.start(), m.group()
        if c in closer:
            stack.append((closer[c], i))
        elif c in ')]}':
            while stack and stack[-1][0] != c:
                found[stack.pop()[1]] = i
            if stack:
                found[stack.pop()[1]] = i + 1
    for _, i in stack:
        found[i] = len(code)
    return found


def matching(code, open_at):
    """The offset just past the brace or bracket that closes CODE's at OPEN_AT."""
    return pairs(code).get(open_at, len(code))


def body_start(code, at):
    """The offset of the `{` opening the body of the item whose head starts
    at AT, or None for one with no body (`;` first).  Brackets, parentheses
    and generic arguments are stepped over, a const argument's braces with
    them (`-> S<{ N + 1 }>`)."""
    depth, angle, i, n = 0, 0, at, len(code)
    while i < n:
        c = code[i]
        if c in '([':
            depth += 1
        elif c in ')]':
            depth -= 1
        elif c == '<' and depth == 0:
            angle += 1
        elif c == '>' and depth == 0 and code[i - 1] not in '-=':
            angle = max(0, angle - 1)
        elif c == '{':
            if depth == 0 and angle == 0:
                return i
            i = matching(code, i)
            continue
        elif c == ';' and depth == 0 and angle == 0:
            return None
        i += 1
    return None


def item_end(code, at):
    """The end of the item or statement starting at AT: its body's close, or
    its `;`; never past the bracket closing a group it is inside."""
    start = body_start(code, at)
    end = matching(code, start) if start is not None else (code.find(';', at) + 1 or len(code))
    depth = 0
    for m in BRACKET.finditer(code, at, end):
        depth += m.group() in '([{'
        depth -= m.group() in ')]}'
        if depth < 0:
            return m.start()
    return end


def requires_test(predicate):
    """Whether a cfg PREDICATE holds only under `test`: `test`, or `all(...)`
    with an argument that does."""
    text = predicate.strip()
    if text == 'test':
        return True
    m = re.fullmatch(r'all\s*\((.*)\)', text, re.DOTALL)
    return bool(m) and any(requires_test(arg) for arg in top_level_args(m[1]))


def top_level_args(text):
    """TEXT's comma-separated arguments, nested parentheses kept whole."""
    args, depth, at = [], 0, 0
    for i, c in enumerate(text):
        depth += c == '('
        depth -= c == ')'
        if c == ',' and depth == 0:
            args.append(text[at:i])
            at = i + 1
    return args + [text[at:]]


def test_attribute(body):
    """Whether an attribute's BODY, between its brackets, makes its item
    test-only: `test`, `tokio::test`, or a cfg that requires test."""
    cfg = CFG.fullmatch(body)
    return bool(TEST_NAME.fullmatch(body) or (cfg and requires_test(cfg[1])))


def attributes(code):
    """[(start, end, body)] of CODE's attributes, `#[` to its `]`."""
    found = []
    for m in ATTR.finditer(code):
        close = matching(code, m.end() - 1)
        found.append((m.start(), close, code[m.end():close - 1]))
    return found


def test_spans(code):
    """The (start, end) spans of CODE's items and statements only tests
    compile: each one a test attribute is on, to its body's close or its `;`.
    A field's or a parameter's attribute owns no span of code counted here."""
    spans = []
    for start, end, body in attributes(code):
        if test_attribute(body):
            after = end
            while (more := NEXT_ATTR.match(code, after)):
                after = matching(code, more.end() - 1)
            if not FIELD.match(code, after):
                spans.append((start, item_end(code, after)))
    return spans


def head(code, at):
    """The offset an item whose keyword is at AT starts at: before its
    qualifiers (`pub(crate)`, `async`, ...) and its attributes, however many
    lines each spans."""
    pos = at
    while True:
        back = pos
        while back and code[back - 1].isspace():
            back -= 1
        if back and code[back - 1] == ']':
            opening = opener(code, back - 1, '[', ']')
            hash_at = opening
            while hash_at > 0 and code[hash_at - 1].isspace():
                hash_at -= 1
            if hash_at > 0 and code[hash_at - 1] == '#':
                pos = hash_at - 1
                continue
        word = back
        while word and (code[word - 1].isalnum() or code[word - 1] == '_'):
            word -= 1
        if word < back and code[word:back] in QUALIFIERS:
            pos = word
            continue
        if back and code[back - 1] == ')':
            opening = opener(code, back - 1, '(', ')')
            keyword = opening
            while keyword and code[keyword - 1].isspace():
                keyword -= 1
            if opening > 0 and code[max(0, keyword - 3):keyword] == 'pub':
                pos = keyword - 3
                continue
        return pos


def opener(code, close, left, right):
    """The offset of the LEFT that the RIGHT at CLOSE closes, or -1."""
    depth = 0
    for i in range(close, -1, -1):
        depth += code[i] == right
        depth -= code[i] == left
        if depth == 0:
            return i
    return -1


def containers(code):
    """[(start, end, name)] of CODE's impl, trait and inline mod blocks: an
    impl named by its type (`T`, or `T as Trait`), the others by their name."""
    found = []
    for m in CONTAINER.finditer(code):
        start = body_start(code, m.end())
        if start is None:
            continue
        header = ' '.join(code[m.end():start].split()).split(' where ')[0]
        if m[1] == 'impl':
            header = without_generics(header)
            trait, _, ty = header.rpartition(' for ')
            name = f'{ty} as {trait}' if trait else header
        else:
            name = header.split('<')[0].split(':')[0].strip()
        found.append((m.start(), matching(code, start), name))
    return found


def without_generics(header):
    """An impl HEADER without its leading generic parameters, `<...>` balanced."""
    if not header.startswith('<'):
        return header
    depth = 0
    for i, c in enumerate(header):
        depth += c == '<'
        depth -= c == '>' and header[i - 1] != '-'
        if depth == 0:
            return header[i + 1:].strip()
    return header


def module_path(path):
    """The module PATH's file is, from its crate: `crates/c/src/a/b.rs' is
    [a, b]; lib.rs, main.rs and mod.rs are their directory's module."""
    parts = list(PurePosixPath(path).parts[2:])
    if parts and parts[0] == 'src':
        parts = parts[1:]
    if parts and parts[-1] in ('lib.rs', 'main.rs', 'mod.rs'):
        parts = parts[:-1]
    elif parts:
        parts[-1] = parts[-1].removesuffix('.rs')
    return parts


def qualified(name, offset, blocks, module=()):
    """NAME inside MODULE, its file's module path, and the BLOCKS
    (containers) that enclose OFFSET, outermost first: a function moved from
    an inline `mod m { }` to m.rs keeps its name."""
    return '::'.join([*module, *(b for start, end, b in blocks if start <= offset < end), name])


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


def test_modules(path, src, code):
    """The files SRC's (at PATH) test-only `mod NAME;' declarations load:
    NAME.rs and NAME/mod.rs beside a mod.rs, lib.rs or main.rs, else in the
    directory named for PATH, or under the inline modules around it (each
    one's own `#[path]' naming its directory); or the file a `#[path]'
    names, beside PATH or under those modules.  Declarations are read from
    CODE, SRC lexed, a `#[path]' value from SRC."""
    p = PurePosixPath(path)
    home = p.parent if p.name in ('mod.rs', 'lib.rs', 'main.rs') else p.parent / p.stem
    marked = {(start, end): body for start, end, body in attributes(code)}
    inline = [(start, end, name) for start, end, name in containers(code) if code.startswith('mod', start)]
    def path_of(at):
        """The `#[path]' value among the attributes of the item at AT, or None."""
        start = head(code, at)
        named = next((PATH_ATTR.match(src, a) for a, b in marked if start <= a and b <= at and PATH_ATTR.match(src, a)),
                     None)
        return named and named[1]

    found = set()
    for m in MOD_DECL.finditer(code):
        enclosing = [(start, name) for start, end, name in inline if start <= m.start() < end]
        within = home
        for n, (start, name) in enumerate(sorted(enclosing)):
            own_path = path_of(start)
            # The outermost one's own #[path] is beside the file, as rustc
            # resolves it before the file's directory is appended.
            base = p.parent if n == 0 and own_path else within
            within = PurePosixPath(posixpath.normpath(str(base / own_path))) if own_path else within / name
        start = head(code, m.start())
        attrs = [(a, b) for (a, b) in marked if start <= a and b <= m.start()]
        if not any(test_attribute(marked[a]) for a in attrs):
            continue
        named = path_of(m.start())
        found |= {posixpath.normpath(str((within if enclosing else p.parent) / named))} if named \
            else {str(within / f'{m[1]}.rs'), str(within / m[1] / 'mod.rs')}
    return found


def file_census(src, lexed=None, module=()):
    """(functions, unwraps, undocumented) of SRC, LEXED its
    unsafe_census.lex: functions as [(name, line, lines, code, declared)],
    the counts outside test items."""
    code, comments, keywords = lexed or unsafe_census.lex(src)
    line_of, note = unsafe_census.annotator(src, code, comments)
    starts = [0] + [m.end() for m in re.finditer('\n', code)]
    uncommented = without(src, comments)
    tests = test_spans(code)
    tokens = [(m.end() - 1, matching(code, m.end() - 1)) for m in TOKEN_MACROS.finditer(code)]
    blocks = containers(code)
    found = []
    for m in FN.finditer(code):
        if inside(tests, m.start()) or inside(tokens, m.start()):
            continue
        start = body_start(code, m.end())
        if start is None:
            continue
        end = matching(code, start)
        first, last = line_of(m.start()), line_of(end - 1)
        found.append((qualified(m.group(2), m.start(), blocks, module), first + 1, last - first + 1,
                      code_lines(uncommented, starts, start + 1, end - 1),
                      bool(MARK.search(note(head(code, m.start()))))))
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
    sources = [(path, text, unsafe_census.lex(text)) for path, text in sorted(sources) if own(path)]
    tests = set().union(*(test_modules(path, text, lex[0]) for path, text, lex in sources))
    for path, text, lex in sources:
        if path in tests:
            continue
        functions, unwraps, undocumented = file_census(text, lex, module_path(path))
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


DECLARE = 'declare it with `// SMELL(long-function): reason` in the comment lines above it'


def rises(base, tree):
    """Why TREE adds to BASE, one line each: both summaries.  Each new long
    function is its own finding."""
    return lisp_functions.rises(base['long'], tree['long'], base['keys'], tree['keys'],
                                LONG_FUNCTION_LINES, KIND, declare=DECLARE, each_new=True) \
        + floor_rises(base['floors'], tree['floors'])


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
