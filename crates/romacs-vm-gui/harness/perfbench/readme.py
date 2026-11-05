#!/usr/bin/env python3
"""Write a perfbench run's results into README.org's Romacs-vs-GNU table.

Usage: readme.py RESULTS.json [README.org]

Replaces the text between the `# perfbench-table-begin' and
`# perfbench-table-end' lines of README.org with an Org table built from
RESULTS.json (perfbench's output), headed by the build, the GNU pin, the
machine and the run's date.  Every row is kept, losses included.
"""
import json
import os
import re
import sys
from pathlib import Path

BEGIN, END = '# perfbench-table-begin', '# perfbench-table-end'


def scaled(value, unit):
    if value is None:
        return '-'
    if unit == 's':
        for limit, scale, suffix in ((1, 1, 's'), (1e-3, 1e3, 'ms'), (1e-6, 1e6, 'µs')):
            if value >= limit:
                return f'{value * scale:.3g} {suffix}'
        return f'{value * 1e9:.3g} ns'
    for limit, suffix in ((1e9, 'G'), (1e6, 'M'), (1e3, 'k')):
        if value >= limit:
            return f'{value / limit:.3g}{suffix}'
    return f'{value:.3g}'


def ratio(value):
    return '-' if value is None else f'{value:.2f}x'


def machine():
    cpu = next((line.split(':', 1)[1].strip() for line in open('/proc/cpuinfo')
                if line.startswith('model name')), 'unknown CPU')
    mem = next((int(line.split()[1]) for line in open('/proc/meminfo')
                if line.startswith('MemTotal')), 0)
    return f'{cpu}, {os.cpu_count()} threads, {round(mem / 1048576)} GB'


def section(results):
    meta = results['meta']
    started = meta['started']
    date = f'{started[0:4]}-{started[4:6]}-{started[6:8]}'
    lines = [
        f'Measured {date} on build ~{meta["commit"][:8]}~ against pinned GNU Emacs '
        f'~{meta["fixtures"]["commit"][:8]}~, on {machine()}.  '
        f'Each workload ran {meta["runs"]} times per engine in private home directories; '
        'wall time is the median, and wall times measured above load '
        f'{meta["max_load"]:g} are left out.  The ratio is Romacs over GNU: below 1.00x, Romacs is faster.',
        '',
        '| Workload | Per | GNU | Romacs | Romacs/GNU (time) | Romacs/GNU (instructions) | Notes |',
        '|-',
    ]
    for r in results['rows']:
        gnu, romacs = r['gnu'] or {}, r['romacs'] or {}
        lines.append(f'| {r["row"]} | {r["unit"]} | {scaled(gnu.get("wall"), "s")} | '
                     f'{scaled(romacs.get("wall"), "s")} | {ratio(r["wall_ratio"])} | '
                     f'{ratio(r["instructions_ratio"])} | {r["note"]} |')
    skipped = [line.split(':', 1)[0] for line in results['skipped']]
    if skipped:
        lines += ['', f'Not measured in this run: {", ".join(skipped)}.']
    return '\n'.join(lines)


def main(argv):
    if not 1 <= len(argv) <= 2:
        raise SystemExit(__doc__)
    results = json.loads(Path(argv[0]).read_text())
    readme = Path(argv[1] if len(argv) == 2 else 'README.org')
    text = readme.read_text()
    pattern = re.compile(rf'^{re.escape(BEGIN)}\n.*?^{re.escape(END)}$', re.S | re.M)
    if not pattern.search(text):
        raise SystemExit(f'{readme}: no {BEGIN} ... {END} block')
    text = pattern.sub(lambda _: f'{BEGIN}\n{section(results)}\n{END}', text)
    readme.write_text(text)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
