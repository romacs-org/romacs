#!/usr/bin/env python3
"""Turn the host boundary allow-list into HOST comments at its sites.

    host_allow.py convert [--root DIR]
    host_allow.py merge BASE OURS THEIRS

The host boundary lint (crates/romacs-vm/tests/host_boundary_lint.rs) read
its exceptions from a list, crates/romacs-vm/tests/host_boundary_allow.txt,
one `PATH ITEM API MARK REASON` per line, MARK the FNV-1a hash of the site
line's tokens. An exception is now a comment in the run of comment lines
directly above its site, naming each API the site's line calls for it:

    // HOST(API, ...): REASON

convert finds each entry's line by its mark, writes the comment there unless
one for that API is there already, and deletes the list. It is idempotent.
A branch that added entries merges the trunk, keeps its own list where git
reports it deleted by the trunk, and runs convert: the branch's new entries
become comments, and the trunk's are there already.

An entry it cannot place is printed, and the list stays: its line is gone,
or several lines of its file share its mark and its item does not tell them
apart. Place those by hand (the lint names each uncovered site), then delete
the list. Delete this tool when no branch forked before the comments is open.

merge is git's driver for the list while it lasts: it merges the list's
entries, keyed by their first four words, so two branches conflict only where
they change one entry differently. `tools/derived setup` installs it.
"""
import argparse
import re
import sys
from pathlib import Path

LIST = 'crates/romacs-vm/tests/host_boundary_allow.txt'
HASHES = re.compile('#*')
TAG = re.compile(r'\s*// HOST\(([^)]*)\):')
WIDTH = 100


def ident(c):
    return c.isascii() and (c.isalnum() or c == '_')


def views(src):
    """(code, text) as host_boundary_lint.rs's `views`: TEXT has comments
    blanked; CODE also blanks string and character literal contents, keeping
    each string's two quotes. Also the line each byte opens inside a string or
    comment, or not."""
    code, text = list(src), list(src)
    inside = [False] * len(src)

    def blank(view, start, end):
        for k in range(start, min(end, len(src))):
            if view[k] != '\n':
                view[k] = ' '

    def mark(start, end):
        for k in range(start, min(end, len(src))):
            inside[k] = True

    i = 0
    while i < len(src):
        rest = src[i:i + 2]
        if rest == '//':
            end = src.find('\n', i)
            end = len(src) if end < 0 else end
            blank(code, i, end), blank(text, i, end)
            i = end
        elif rest == '/*':
            depth, j = 0, i
            while j < len(src):
                if src.startswith('/*', j):
                    depth, j = depth + 1, j + 2
                elif src.startswith('*/', j):
                    depth, j = depth - 1, j + 2
                    if depth == 0:
                        break
                else:
                    j += 1
            blank(code, i, j), blank(text, i, j), mark(i + 1, j)
            i = j
        elif (raw := raw_string_start(src, i)) is not None:
            opening, hashes = raw
            close = next((j for j in range(opening + 1, len(src))
                          if src[j] == '"' and src[j + 1:j + 1 + hashes].count('#') == hashes), len(src) - 1)
            blank(code, i, close + 1 + hashes), mark(i + 1, close + 1)
            code[opening], code[close] = '"', '"'
            i = close + 1 + hashes
        elif src[i] == '"':
            j = i + 1
            while j < len(src) and src[j] != '"':
                j += 2 if src[j] == '\\' else 1
            blank(code, i + 1, j), mark(i + 1, j + 1)
            i = j + 1
        elif src[i] == "'":
            end = char_literal_end(src, i)
            if end is None:
                i += 1
            else:
                blank(code, i, end)
                i = end
        elif ident(src[i]):
            while i < len(src) and ident(src[i]):
                i += 1
        else:
            i += 1
    return ''.join(code), ''.join(text), inside


def raw_string_start(s, i):
    if i > 0 and ident(s[i - 1]):
        return None
    j = i + (s[i:i + 1] == 'b')
    if s[j:j + 1] != 'r':
        return None
    j += 1
    hashes = HASHES.match(s, j).end() - j
    return (j + hashes, hashes) if s[j + hashes:j + hashes + 1] == '"' else None


def char_literal_end(s, i):
    if s[i + 1:i + 2] == '\\':
        j = i + 3
        while j < len(s) and s[j] != "'":
            j += 1
        return j + 1
    if i + 1 >= len(s):
        return None
    return i + 3 if s[i + 2:i + 3] == "'" else None


def marks(src):
    """{line: mark} of SRC, 1-based: the hash of the line's tokens, a space only
    between two words of it, then its literal contents, as the lint keyed an
    entry."""
    code, text, _ = views(src)
    line_of, n = [], 1
    for c in code:
        line_of.append(n)
        n += c == '\n'
    spelt, last, i = {}, None, 0
    while i < len(code):
        c = code[i]
        if c.isspace():
            i += 1
            continue
        here = line_of[i]
        if ident(c):
            j = i
            while j < len(code) and ident(code[j]):
                j += 1
            word, tok, i = True, text[i:j], j
        elif c == '"':
            close = code.find('"', i + 1)
            close = len(code) - 1 if close < 0 else close
            word, tok, i = True, text[i:close + 1], close + 1
        else:
            word, tok, i = False, c, i + 1
        out = spelt.setdefault(here, [])
        if word and last == (here, True):
            out.append(' ')
        last = (here, word)
        out.append(tok)
    for at, (t, k) in enumerate(zip(text, code)):
        if k == ' ' and not t.isspace():
            spelt.setdefault(line_of[at], []).append(t)
    return {line: fnv(''.join(parts).encode()) for line, parts in spelt.items()}


def fnv(data):
    h = 0xcbf29ce484222325
    for b in data:
        h = ((h ^ b) * 0x100000001b3) & 0xffffffffffffffff
    return f'{h:016x}'


def entries(text):
    """[(path, item, api, mark, reason)] of the list TEXT."""
    out = []
    for line in text.splitlines():
        if line.strip() and not line.startswith('#'):
            path, item, api, mark, *reason = line.split()
            out.append((path, item, api, mark, ' '.join(reason)))
    return out


ITEM = re.compile(r'\b(?:fn|const|static(?:\s+mut)?|mod|struct|enum|trait|type)\s+(\w+)|macro_rules!\s*(\w+)')


def enclosing(lines, line):
    """The name of the nearest item declared at or above LINE (1-based)."""
    for row in range(line - 1, -1, -1):
        found = ITEM.search(lines[row])
        if found:
            return found.group(1) or found.group(2) + '!'
    return '-'


def site(lines, line_marks, item, mark):
    """The line of the entry with ITEM and MARK, or None."""
    lines_marked = [n for n, m in line_marks.items() if m == mark]
    if len(lines_marked) > 1:
        last = item.rsplit('::', 1)[-1]
        lines_marked = [n for n in lines_marked if enclosing(lines, n) == last]
    return lines_marked[0] if len(lines_marked) == 1 else None


def comment_run(lines, line):
    """The comment-only lines directly above LINE (1-based), nearest last."""
    row = line - 1
    while row > 0 and lines[row - 1].lstrip().startswith('//'):
        row -= 1
    return lines[row:line - 1]


def tagged(lines, line, api):
    """Whether a HOST comment directly above LINE names API."""
    return any(api in apis for l in comment_run(lines, line) if (apis := tag(l)))


def tag(line):
    """The APIs a `// HOST(A, B): REASON` comment line names, or []."""
    found = TAG.match(line)
    return [a.strip() for a in found.group(1).split(',')] if found else []


def comment(indent, apis, reason):
    """REASON for APIS as `// HOST(A, B): …` lines at INDENT, wrapped at WIDTH."""
    head = f'// HOST({", ".join(apis)}):'
    out = [indent + head]
    for word in reason.split():
        if len(out[-1]) + 1 + len(word) > WIDTH and out[-1].strip() != head:
            out.append(f'{indent}//')
        out[-1] += ' ' + word
    return [l + '\n' for l in out]


def convert(root, listed):
    """Write each entry of LISTED as a comment under ROOT; the entries it
    could not place, each with why."""
    by_path, failed = {}, []
    for entry in listed:
        by_path.setdefault(entry[0], []).append(entry)
    for path, group in sorted(by_path.items()):
        file = root / path
        if not file.is_file():
            failed.extend((e, 'no such file') for e in group)
            continue
        src = file.read_text()
        lines = src.splitlines(keepends=True)
        line_marks = marks(src)
        inside = views(src)[2]
        starts = [0]
        for l in lines:
            starts.append(starts[-1] + len(l))
        inserts = {}
        for entry in group:
            _, item, api, mark, reason = entry
            line = site(lines, line_marks, item, mark)
            if line is None:
                failed.append((entry, 'no line, or no one line, has its mark'))
            elif inside[starts[line - 1]] if starts[line - 1] < len(src) else False:
                failed.append((entry, f'line {line} opens inside a literal or comment'))
            elif not tagged(lines, line, api) and (api, reason) not in inserts.get(line, []):
                inserts.setdefault(line, []).append((api, reason))
        for line in sorted(inserts, reverse=True):
            indent = re.match(r'\s*', lines[line - 1]).group()
            by_reason = {}
            for api, reason in sorted(inserts[line]):
                by_reason.setdefault(reason, []).append(api)
            block = [l for reason, apis in by_reason.items() for l in comment(indent, apis, reason)]
            lines[line - 1:line - 1] = block
        if inserts:
            file.write_text(''.join(lines))
    return failed


KEY = 4


def keyed(text):
    """{key: line} of the list TEXT; ValueError on a key listed twice."""
    out = {}
    for line in text.splitlines():
        if line.strip() and not line.startswith('#'):
            key = tuple(line.split()[:KEY])
            if key in out:
                raise ValueError(f'{" ".join(key)} is listed twice')
            out[key] = line
    return out


def merged(base, ours, theirs):
    """(text, conflicts): the three-way merge of three lists' texts, entry by
    entry, under OURS's comment lines and in the order the lint writes, and
    the keys the sides changed differently. Such an entry keeps each side's
    line it has, which the lint then reports (listed twice, or stale)."""
    b, o, t = keyed(base), keyed(ours), keyed(theirs)
    out = [line for line in ours.splitlines() if line.startswith('#')]
    conflicts = []
    for key in sorted(set(b) | set(o) | set(t)):
        was, mine, yours = b.get(key), o.get(key), t.get(key)
        if mine != was and yours != was and mine != yours:
            conflicts.append(' '.join(key))
            out.extend(line for line in (mine, yours) if line is not None)
        elif (line := yours if mine == was else mine) is not None:
            out.append(line)
    return '\n'.join(out) + '\n', conflicts


def merge_files(base, ours, theirs):
    """Write the merge over OURS: 0, or 1 when the sides changed an entry
    differently, so git leaves the file unmerged with both its lines in it.
    1, leaving OURS, when a side lists a key twice."""
    try:
        text, conflicts = merged(*(Path(p).read_text() for p in (base, ours, theirs)))
    except (OSError, ValueError) as error:
        print(f'host_allow: {error}', file=sys.stderr)
        return 1
    Path(ours).write_text(text)
    for key in conflicts:
        print(f'host_allow: {key} changed differently on the two sides', file=sys.stderr)
    return 1 if conflicts else 0


def main(argv):
    if argv[:1] == ['merge']:
        return merge_files(*argv[1:]) if len(argv) == 4 else 2
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('action', choices=('convert',))
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args(argv)
    path = args.root / LIST
    if not path.exists():
        print(f'host_allow: no {LIST}; nothing to convert')
        return 0
    failed = convert(args.root, entries(path.read_text()))
    for entry, why in failed:
        print(f'host_allow: {why}: {" ".join(entry[:4])}', file=sys.stderr)
    if failed:
        print(f'host_allow: {len(failed)} entries unplaced; {LIST} stays', file=sys.stderr)
        return 1
    path.unlink()
    print(f'host_allow: every entry is a comment; deleted {LIST}')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
