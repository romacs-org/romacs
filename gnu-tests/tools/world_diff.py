#!/usr/bin/env python3
"""world_diff.py A B [--allow FILE]: compare two world-digest.el outputs.

Each input holds ROOT TAB NAME TAB DIGEST lines. Every root present in only
one, or digested differently, is listed. FILE names the roots allowed to
differ, one `ROOT TAB NAME TAB REASON' per line: values two fresh processes
also give differently (the clock, the random state). Exit 0 when every
difference is allowed, 1 otherwise; an allowed root that does not differ is
listed too, so the list cannot rot unnoticed.
"""
import argparse
import sys


def roots(path):
    out = {}
    with open(path, encoding='utf-8') as f:
        for line in f:
            root, name, digest = line.rstrip('\n').split('\t')
            out[(root, name)] = digest
    return out


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
    roots that did not differ."""
    keys = sorted(set(a) | set(b))
    differ = [k for k in keys if a.get(k) != b.get(k)]
    return ([k for k in differ if k not in allow], [k for k in differ if k in allow],
            [k for k in allow if k not in differ])


def main(argv):
    ap = argparse.ArgumentParser()
    ap.add_argument('a')
    ap.add_argument('b')
    ap.add_argument('--allow')
    args = ap.parse_args(argv)
    a, b, allow = roots(args.a), roots(args.b), allowed(args.allow)
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
