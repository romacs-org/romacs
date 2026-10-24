#!/usr/bin/env python3
"""Render the GUI's loadup.el table (romacs-vm-gui --loadup-report) as markdown.

    table.py loadup.tsv > table.md

One row per load site of lisp/loadup.el in loadup.el's own order, plus one row
per top-level loadup.el form that signalled, placed at its source line. The
status column is the VM's measured outcome; nothing here is inferred.
"""
import collections, sys

rows = [l.rstrip('\n').split('\t') for l in open(sys.argv[1]) if l.strip()]
head, rows = rows[0], rows[1:]
assert head[:6] == ['site', 'line', 'file', 'status', 'forms', 'detail'], head
rows.sort(key=lambda r: (int(r[1]), 0 if r[0].isdigit() else 1))
count = collections.Counter(r[3] for r in rows if r[0].isdigit())
print(f"{sum(count.values())} load sites: " + ', '.join(f'{k} {v}' for k, v in sorted(count.items())))
print(f"{sum(1 for r in rows if not r[0].isdigit())} top-level loadup.el forms signalled\n")
print('| site | loadup.el line | file | outcome | forms run / total | stop cause (first failing form) |')
print('|---|---|---|---|---|---|')
for r in rows:
    detail = r[5].replace('|', '\\|').replace('\n', ' ')
    if len(detail) > 260: detail = detail[:257] + '...'
    print(f"| {r[0]} | {r[1]} | `{r[2]}` | {r[3]} | {r[4]} | {detail} |")
