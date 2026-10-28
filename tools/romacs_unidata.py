#!/usr/bin/env python3
"""Derive Romacs's byte-code-free uni-*.el tables from GNU's (VM2-UNIDATA1).

    romacs_unidata.py SOURCE-DIR DEST-DIR

GNU's unidata-gen.el writes some Unicode property functions into its
generated tables as byte code. Romacs runs Lisp from source, so each
top-level `#[...]' literal in those tables is replaced by the name of the
same function in rlisp/romacs-unidata.el, and the file requires that
library first. Every other byte is GNU's. SOURCE-DIR is lisp/international
of the pinned GNU build (45b0f76); each input's SHA-256 is checked.
"""
import hashlib
import pathlib
import sys

# The pinned build's SHA-256 and, in order, the functions its byte-code literals are.
TABLES = {
    'uni-name.el': ('abcdf28f6726683ed140b1280f6ffdd816371e88146dc90a445fd9819da6517d', ['get-name', 'put-name']),
    'uni-old-name.el': ('f430de85accc2ad5fa8ea6f0d374f3635acd772b33a4922756c6ef3f1b48ea4d', ['get-name', 'put-name']),
    'uni-comment.el': ('8bf4664418c780ed18a715842f4f56e0e15b42c1676ed35f116d509482089a8d', ['get-name', 'put-name']),
    'uni-decomposition.el': ('24e0acf60bd025a94401e2ed650138735e8674dee0ace415d293b74bb742103f',
                             ['get-decomposition', 'put-decomposition', 'describe-decomposition']),
    'uni-category.el': ('21ddd35e81a9c73d209b4ced985dc0b2ef39ab7d7aa7f55870576d38661ed7cd', ['describe-general-category']),
    'uni-combining.el': ('868acd4308fceec9e1105b1e54c8be076542e9c95578b88fa7df90798a919a08',
                         ['describe-canonical-combining-class']),
    'uni-bidi.el': ('9cb55db6ba7712773f08d0acedb1034556d1fcbd42ece91a1397a4390e05a871', ['describe-bidi-class']),
    'uni-brackets.el': ('2b244935d292b542f7c310039fd5499c4a2968582b36a453e1553cb38db40671', ['describe-bidi-bracket-type']),
}
# A get or put function takes (CHAR VAL TABLE); a describer takes (VAL).
ARITY = {b'771': ('get-', 'put-'), b'257': ('describe-',)}
FORM = b'(define-char-code-property '
NOTE = b';; romacs: GNU byte code replaced by rlisp/romacs-unidata.el functions (tools/romacs_unidata.py).\n'


def byte_code_spans(text):
    """(start, end) of each top-level #[...] literal, skipping strings and escapes."""
    spans, depth, start, i = [], 0, None, 0
    while i < len(text):
        c = text[i:i + 1]
        if c == b'"':
            i += 1
            while text[i:i + 1] != b'"':
                i += 2 if text[i:i + 1] == b'\\' else 1
        elif c == b'\\':
            i += 1
        elif c == b'[':
            if depth == 0 and text[i - 1:i] == b'#':
                start = i - 1
            if start is not None:
                depth += 1
        elif c == b']' and start is not None:
            depth -= 1
            if depth == 0:
                spans.append((start, i + 1))
                start = None
        i += 1
    return spans


def derive(name, text):
    digest, functions = TABLES[name]
    if hashlib.sha256(text).hexdigest() != digest:
        raise SystemExit(f'romacs_unidata: {name} is not the pinned build\'s output')
    spans = byte_code_spans(text)
    if len(spans) != len(functions):
        raise SystemExit(f'romacs_unidata: {name}: {len(spans)} byte-code literals, expected {len(functions)}')
    out, at = [], 0
    for (start, end), function in zip(spans, functions):
        arity = text[start + 2:start + 5]
        if not any(function.startswith(prefix) for prefix in ARITY.get(arity, ())):
            raise SystemExit(f'romacs_unidata: {name}: literal at {start} has arity {arity!r}, not {function}')
        out += [text[at:start], b'romacs-unidata-' + function.encode()]
        at = end
    out.append(text[at:])
    text = b''.join(out)
    form = text.index(FORM)
    first_line = text.index(b'\n') + 1
    return text[:first_line] + NOTE + text[first_line:form] + b"(require 'romacs-unidata)\n" + text[form:]


def main():
    source, dest = map(pathlib.Path, sys.argv[1:3])
    for name in TABLES:
        (dest / name).write_bytes(derive(name, (source / name).read_bytes()))


if __name__ == '__main__':
    main()
