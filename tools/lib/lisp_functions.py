#!/usr/bin/env python3
"""The functions of Romacs's own Lisp, and how long each is.

A function is a definition form (`DEFINERS`: defun, defmacro, cl-defmethod, a
mode's body, ...) found at any depth outside another one, so a defun inside
`with-eval-after-load' counts.  Its length is its code lines: the lines of the
form that hold code, as Clippy's too_many_lines counts a Rust function's.  A
line holding only blanks, a comment, the docstring, or literal data (a quoted
list or vector, a backquoted one without an unquote) is not code; literal data
is counted apart (`data`).  `lines` is the form's whole span.

A deliberately long function is declared in the comment lines directly above
it: `;; SMELL(long-function): reason`.

Romacs's own Lisp is lisp/bundled.json's: its subsystem directories, its
off-path halves, and `romacs-*' files elsewhere under lisp/, never a bundled
package.  tools/lib/smells.py holds the count over LONG_FUNCTION_LINES to the
merge-base's, and no function already over it may grow.

usage: lisp_functions.py report [--root ROOT] [--over N]   the functions spanning N lines or more, by code lines
"""
import argparse
import bisect
import json
import re
import subprocess
import sys
from collections import namedtuple
from pathlib import Path, PurePosixPath

LONG_FUNCTION_LINES = 40
# The ratchet warns before it holds: its findings fail a check only once
# ENFORCING is true, from ENFORCED_FROM on, so the branches in flight when it
# came are told, not bounced.
ENFORCING = False
ENFORCED_FROM = '2026-11-23 06:00'
KIND = 'long_functions'
DEFINERS = frozenset(('defun', 'defmacro', 'defsubst', 'cl-defun', 'cl-defmacro', 'cl-defsubst',
                      'cl-defmethod', 'cl-defgeneric', 'cl-iter-defun', 'iter-defun', 'define-inline',
                      'define-minor-mode', 'define-globalized-minor-mode', 'define-derived-mode'))
# Where a definer's docstring is, when not after its argument list (the fourth element).
DOC_AT = {'define-minor-mode': 2, 'define-globalized-minor-mode': 4, 'define-derived-mode': 4}
LAYOUT = 'lisp/bundled.json'
MARK = re.compile(r'\bSMELL\(long-function\):[ \t]*\S')

# A node of the reader: KIND is 'list' (a vector, record or propertized string
# too: TEXT is its opener), 'atom' (TEXT is its name) or 'string'; START and END
# are offsets into the text; PREFIX the reader macros before it; KIDS a list's elements.
Node = namedtuple('Node', 'kind start end prefix kids text')
Function = namedtuple('Function', 'name kind line lines code data declared')

DELIMITERS = set('()[]";\'`,') | set(' \t\n\r\f')
CLOSER = {'(': ')', '#(': ')', '#s(': ')', '[': ']', '#[': ']'}


class ReadError(ValueError):
    pass


def skip_symbol(text, i):
    """The end of the symbol (or number) starting at I: `\\` escapes a character."""
    n = len(text)
    while i < n and text[i] not in DELIMITERS:
        i += 2 if text[i] == '\\' else 1
    return min(i, n)


def skip_string(text, i):
    """The end of the string whose opening quote is at I."""
    i += 1
    n = len(text)
    while i < n and text[i] != '"':
        i += 2 if text[i] == '\\' else 1
    if i >= n:
        raise ReadError(f'unterminated string at offset {i}')
    return i + 1


def skip_char(text, i):
    """The end of the character literal whose `?` is at I: its modifiers
    (`?\\C-\\M-x', `?\\^x'), one character or an escape, then what a symbol
    may hold (`?\\N{NAME}', `?\\u00e9')."""
    i += 1
    while re.match(r'\\[CMSHsA]-|\\\^', text[i:i + 3]):
        i += 2 if text[i + 1] == '^' else 3
    i += 2 if text.startswith('\\', i) else 1
    if text[i - 1] == 'N' and text.startswith('{', i):
        i = text.find('}', i) + 1 or len(text)
    return skip_symbol(text, min(i, len(text)))


OPENERS = ('#s(', '#(', '#[', '(', '[')
PREFIXES = (",@", "#'", "'", '`', ',')


def token(text, i):
    """(KIND, END) of the token at I of TEXT, past blanks and comments: KIND is
    'open' (then END covers the opener), 'close', 'prefix', 'string' or 'atom'."""
    c = text[i]
    if text.startswith(OPENERS, i):
        return 'open', i + next(len(o) for o in OPENERS if text.startswith(o, i))
    if c in ')]':
        return 'close', i + 1
    if text.startswith(PREFIXES, i):
        return 'prefix', i + next(len(p) for p in PREFIXES if text.startswith(p, i))
    if c == '"':
        return 'string', skip_string(text, i)
    if c == '?':
        return 'atom', skip_char(text, i)
    if text.startswith('#@', i):
        raise ReadError(f'#@ at offset {i} is not read here')
    return 'atom', max(skip_symbol(text, i + 1 if c == '#' else i), i + 1)


def tokens(text):
    """(KIND, START, END) of each token of TEXT, comments and blanks dropped."""
    i, n = 0, len(text)
    while i < n:
        if text[i] in ' \t\n\r\f':
            i += 1
        elif text[i] == ';':
            i = n if text.find('\n', i) < 0 else text.find('\n', i)
        else:
            kind, end = token(text, i)
            yield kind, i, end
            i = end


def read(text):
    """The top-level nodes of Elisp TEXT; ReadError when it does not balance."""
    stack, opened, prefix = [[]], [], ''
    for kind, start, end in tokens(text):
        if kind == 'prefix':
            prefix += text[start:end]
            continue
        if kind == 'open':
            opened.append((start, prefix, text[start:end]))
            stack.append([])
        elif kind == 'close':
            if not opened or CLOSER[opened[-1][2]] != text[start]:
                raise ReadError(f'unbalanced {text[start]!r} at offset {start}')
            begin, pre, opener = opened.pop()
            kids = stack.pop()
            stack[-1].append(Node('list', begin, end, pre, kids, opener))
        else:
            stack[-1].append(Node(kind, start, end, prefix, (), text[start:end]))
        prefix = ''
    if opened:
        raise ReadError(f'unclosed list at offset {opened[-1][0]}')
    return stack[0]


def head(node):
    """The symbol NODE, a list, starts with, or None."""
    if node.kind == 'list' and node.text == '(' and node.kids and node.kids[0].kind == 'atom':
        return node.kids[0].text
    return None


def unquoted(node):
    """Whether NODE, inside a backquote, evaluates something: an unquote in it."""
    return ',' in node.prefix or any(unquoted(k) for k in node.kids)


def literal(node):
    """Whether NODE is literal data: a quoted list or vector, a backquoted one
    without an unquote, a vector, or `(quote X)'."""
    if node.kind != 'list':
        return False
    if "'" in node.prefix.replace("#'", ''):
        return True
    if '`' in node.prefix:
        return not any(unquoted(k) for k in node.kids)
    return node.text != '(' or head(node) == 'quote'


def docstring(form):
    """The docstring node of the definition FORM, or None."""
    kind = head(form)
    at = DOC_AT.get(kind, 3)
    if kind == 'cl-defmethod':
        at = next((j + 1 for j, k in enumerate(form.kids[2:], 2) if k.kind == 'list'), len(form.kids))
    body = 0 if kind in DOC_AT or kind == 'cl-defgeneric' else 1     # a defun's lone string is its value
    return form.kids[at] if at + body < len(form.kids) and form.kids[at].kind == 'string' else None


def spanned(line_of, node):
    """The lines NODE spans."""
    return range(line_of(node.start), line_of(node.end - 1) + 1)


def code_and_data(line_of, form, doc):
    """(code lines, data lines) of FORM, its DOC node aside.  A list holds code
    on its first and last line; its elements say what the lines between hold."""
    code, data = set(), set()
    pending = [form]
    while pending:
        node = pending.pop()
        if node is doc:
            continue
        if literal(node):
            data.update(spanned(line_of, node))
        elif node.kind == 'list':
            code.update((line_of(node.start), line_of(node.end - 1)))
            pending.extend(node.kids)
        else:
            code.update(spanned(line_of, node))
    return code, data - code


def line_index(text):
    """A function from an offset of TEXT to its 1-based line."""
    starts = [0] + [m.end() for m in re.finditer('\n', text)]
    return lambda offset: bisect.bisect_right(starts, offset)


def declared(lines, first):
    """Whether the comment lines directly above line FIRST declare the function long."""
    j = first - 2
    while j >= 0 and lines[j].lstrip().startswith(';'):
        if MARK.search(lines[j]):
            return True
        j -= 1
    return False


def name_of(form):
    """The name of definition FORM: its symbol, a method's with its qualifiers and arguments."""
    if len(form.kids) < 2:
        return '?'
    name = written(form.kids[1])
    if head(form) != 'cl-defmethod':
        return name
    at = next((j for j, k in enumerate(form.kids[2:], 2) if k.kind == 'list'), len(form.kids))
    dispatch = [written(k) for k in form.kids[2:at]] + [specializers(form.kids[at])] if at < len(form.kids) else []
    return ' '.join([name, *dispatch])


def specializers(args):
    """A method's dispatch: its required arguments' specializers and its
    `&context' ones: `((x string) y &optional z)' -> `(string t)'."""
    kept, section = [], 'required'
    for k in args.kids:
        if k.kind == 'atom' and k.text.startswith('&'):
            section = k.text
            kept += [k.text] if section == '&context' else []
        elif section == 'required':
            kept.append(written(k.kids[1]) if k.kind == 'list' and len(k.kids) > 1 else 't')
        elif section == '&context':
            kept.append(written(k))
    return '(' + ' '.join(kept) + ')'


def written(node):
    """NODE as one line of text, its spacing and comments dropped."""
    if node.kind != 'list':
        return node.prefix + node.text
    return node.prefix + node.text + ' '.join(written(k) for k in node.kids) + CLOSER[node.text]


def definitions(nodes):
    """The outermost definition forms among NODES and their descendants."""
    found, pending = [], list(reversed(nodes))
    while pending:
        node = pending.pop()
        if node.kind == 'list' and not literal(node):
            if head(node) in DEFINERS:
                found.append(node)
            else:
                pending.extend(reversed(node.kids))
    return found


def functions(text):
    """TEXT's functions, each a Function; ReadError when TEXT does not read."""
    nodes = read(text)
    line_of = line_index(text)
    lines = text.splitlines()
    out = []
    for form in definitions(nodes):
        first, last = line_of(form.start), line_of(form.end - 1)
        code, data = code_and_data(line_of, form, docstring(form))
        out.append(Function(name_of(form), head(form), first, last - first + 1, len(code), len(data),
                            declared(lines, first)))
    return out


def own(layout, path):
    """Whether PATH, from the root, is Romacs's own Lisp by LAYOUT (lisp/bundled.json's data)."""
    p = PurePosixPath(path)
    if p.suffix != '.el' or p.parts[:1] != ('lisp',) or len(p.parts) < 2:
        return False
    top = p.parts[1] if len(p.parts) > 2 else None
    if top == layout['packages']:
        return False
    return top in {*layout['directories'], *layout['off_path']} or p.name.startswith('romacs-')


def census(layout, sources):
    """({key: (path, Function)}, [what could not be read]) of the own files among
    SOURCES, (path, text) pairs.  A key is `PATH NAME', a second definition of
    a name in one file `PATH NAME#2'."""
    found, errors = {}, []
    for path, text in sorted(sources):
        if not own(layout, path):
            continue
        try:
            for f in functions(text):
                key, k = f'{path} {f.name}', 1
                while key in found:
                    k += 1
                    key = f'{path} {f.name}#{k}'
                found[key] = (path, f)
        except ReadError as exc:
            errors.append(f'{path}: {exc}')
    return found, errors


def long_functions(found, limit=LONG_FUNCTION_LINES):
    """{key: [code lines, 'path:line', name]} of FOUND's undeclared functions over LIMIT code lines."""
    return {key: [f.code, f'{path}:{f.line}', f.name] for key, (path, f) in found.items()
            if f.code > limit and not f.declared}


DECLARE = 'declare it with `;; SMELL(long-function): reason` in the comment lines above it'


def by_name(long):
    """{name: LONG's entries of that name, longest first}."""
    names = {}
    for entry in sorted(long.values(), key=lambda e: -e[0]):
        names.setdefault(entry[2], []).append(entry)
    return names


def paired(base, tree, base_keys, tree_keys):
    """[(tree entry, base entry)] of the long functions TREE and BASE share: by
    key (file and name) first; then a function that moved, whose key the
    other side has no function at (BASE_KEYS, TREE_KEYS: every key, short
    ones too), by name, longest first.  A tree entry left unpaired is new."""
    pairs = [(tree[key], base[key]) for key in sorted(tree) if key in base]
    had = by_name({key: entry for key, entry in base.items() if key not in tree_keys})
    rest = by_name({key: entry for key, entry in tree.items() if key not in base_keys})
    return pairs + [pair for name, entries in sorted(rest.items()) for pair in zip(entries, had.get(name, []))]


def rises(base, tree, base_keys=None, tree_keys=None, limit=LONG_FUNCTION_LINES):
    """Why TREE adds to BASE's long functions, one line each: both
    long_functions; BASE_KEYS and TREE_KEYS every function's key on each
    side, default the long ones'."""
    pairs = paired(base, tree, set(base) if base_keys is None else set(base_keys),
                   set(tree) if tree_keys is None else set(tree_keys))
    grown = [f'{KIND}: {place}: {name} grew to {code} code lines from {was[0]}; '
             f'fix: move a job out of it into a function of its own, or {DECLARE}'
             for (code, place, name), was in pairs if code > was[0]]
    if len(tree) > len(base):
        old = {id(entry) for entry, _ in pairs}
        new = sorted((entry for entry in tree.values() if id(entry) not in old), key=lambda e: e[1])
        grown.append(f'{KIND}: {len(tree)} over {limit} code lines, above the {len(base)} of the '
                     f'merge-base; new: {", ".join(f"{name} ({place})" for _, place, name in new)}; '
                     f'fix: split each into functions of one job, or {DECLARE}')
    return grown


def held(findings):
    """(failing, warned) of a check's FINDINGS: the long-function ones only
    warn until ENFORCING."""
    if ENFORCING:
        return list(findings), []
    ours = [f for f in findings if f.startswith(f'{KIND}: ')]
    return [f for f in findings if f not in ours], ours


# Reading a tree: the working tree, or a commit's objects.

def git(root, *args):
    return subprocess.run(['git', '-C', str(root), *args], check=True, stdout=subprocess.PIPE, text=True).stdout


def working_sources(root, layout):
    """(path, text) of the own Lisp git has, or would have, in the working tree at ROOT."""
    names = git(root, 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', 'lisp').split('\0')
    files = [(name, Path(root) / name) for name in names if name and own(layout, name)]
    return [(name, path.read_text(encoding='utf-8', errors='replace')) for name, path in files if path.is_file()]


def commit_sources(root, commit):
    """(layout, [(path, text)]) of the own Lisp in COMMIT, read from git's objects;
    a tree without lisp/bundled.json has none."""
    shown = subprocess.run(['git', '-C', str(root), 'show', f'{commit}:{LAYOUT}'], stdout=subprocess.PIPE,
                           stderr=subprocess.DEVNULL, text=True)
    if shown.returncode:
        return None, []
    layout = json.loads(shown.stdout)
    listed = git(root, 'ls-tree', '-r', '-z', commit, '--', 'lisp').split('\0')
    blobs = [(entry.split('\t', 1)[1], entry.split()[2]) for entry in listed
             if entry and entry.split()[1] == 'blob' and own(layout, entry.split('\t', 1)[1])]
    batch = subprocess.run(['git', '-C', str(root), 'cat-file', '--batch'], check=True, stdout=subprocess.PIPE,
                           input=''.join(f'{sha}\n' for _, sha in blobs).encode()).stdout
    out, at = [], 0
    for path, _ in blobs:
        header_end = batch.index(b'\n', at)
        size = int(batch[at:header_end].split()[2])
        out.append((path, batch[header_end + 1:header_end + 1 + size].decode('utf-8', 'replace')))
        at = header_end + 1 + size + 1
    return layout, out


def tree_census(root):
    """census of the working tree at ROOT; a tree without lisp/bundled.json has no own Lisp."""
    path = Path(root) / LAYOUT
    if not path.is_file():
        return {}, []
    layout = json.loads(path.read_text())
    return census(layout, working_sources(root, layout))


def commit_census(root, commit):
    """census of COMMIT's tree."""
    return census(*commit_sources(root, commit))


def report(found, over):
    rows = sorted(((f.code, key, path, f) for key, (path, f) in found.items() if f.lines >= over),
                  key=lambda r: (-r[0], r[1]))
    return '\n'.join(f'{f.code:>5} {f.lines:>5} {f.data:>5}{" D" if f.declared else "  "} {f.name}  {path}:{f.line}'
                     for _, _, path, f in rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=['report'])
    parser.add_argument('--root', default='.')
    parser.add_argument('--over', type=int, default=LONG_FUNCTION_LINES + 1)
    args = parser.parse_args()
    found, errors = tree_census(args.root)
    for error in errors:
        print(f'lisp_functions: {error}', file=sys.stderr)
    print(f'{"code":>5} {"lines":>5} {"data":>5}   function  file:line')
    print(report(found, args.over))
    return 1 if errors else 0


if __name__ == '__main__':
    sys.exit(main())
