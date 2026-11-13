#!/usr/bin/env python3
"""world_diff.py A B [--allow FILE]: compare two world-digest.el outputs.

Each input holds ROOT TAB NAME TAB DIGEST lines and ends with `end TAB
lines TAB N', N counting the lines before it: an input without it, or with
another count, was cut short, and is refused. Every root present in only
one, or digested differently, is listed. FILE names the roots whose
digests may differ, one `ROOT TAB NAME TAB REASON' per line: values two
fresh processes also give differently (the clock, the random state). An
allowed root must be present in both: an allowance excuses a value, never
a root gone. Exit 0 when every difference is allowed, 1 otherwise; an
allowed root that does not differ is listed too, so the list cannot rot
unnoticed.
"""
import argparse
import sys


class Incomplete(Exception):
    """A digest without its end line, or with a count that does not match."""


def parse(lines):
    """The roots of a digest's LINES, refused unless complete."""
    out = {}
    if not lines or not lines[-1].startswith('end\tlines\t'):
        raise Incomplete('no end line')
    *body, end = lines
    if end != f'end\tlines\t{len(body)}':
        raise Incomplete(f'{end!r} for {len(body)} lines')
    if not body:
        raise Incomplete('no roots')
    for line in body:
        # A name may hold a tab; the root and the digest hold none.
        rest, digest = line.rsplit('\t', 1)
        root, name = rest.split('\t', 1)
        if (root, name) in out:
            raise Incomplete(f'{root} {name} twice')
        out[(root, name)] = digest
    return out


def roots(path):
    with open(path, encoding='utf-8') as f:
        return parse(f.read().splitlines())


def allowed(path):
    out = {}
    if path:
        with open(path, encoding='utf-8') as f:
            for line in f:
                if line.strip() and not line.startswith('#'):
                    root, name, reason = line.rstrip('\n').split('\t', 2)
                    out[(root, name)] = reason
    return out


def compare(a, b, allow):
    """The differing roots, the allowed ones among them, and the allowed
    roots that did not differ. A root present in only one differs, allowed
    or not."""
    keys = sorted(set(a) | set(b))
    differ = [k for k in keys if a.get(k) != b.get(k)]
    excused = [k for k in differ if k in allow and k in a and k in b]
    return ([k for k in differ if k not in excused], excused, [k for k in allow if k not in differ])


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument('a')
    ap.add_argument('b')
    ap.add_argument('--allow')
    args = ap.parse_args(argv)
    try:
        a, b = roots(args.a), roots(args.b)
    except Incomplete as e:
        print(f'world-diff: incomplete digest: {e}')
        return 2
    allow = allowed(args.allow)
    bad, ok, unused = compare(a, b, allow)
    print(f'world-diff: {len(a)} and {len(b)} roots; {len(bad)} differ, {len(ok)} allowed, {len(unused)} allowed but equal')
    for root, name in bad:
        side = 'only A' if (root, name) not in b else 'only B' if (root, name) not in a else 'differs'
        print(f'  DIFF {root} {name} ({side})')
    for root, name in ok:
        print(f'  allowed {root} {name}: {allow[(root, name)]}')
    for root, name in unused:
        print(f'  equal-but-allowed {root} {name}')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
