#!/usr/bin/env python3
"""Write a perfbench run's results into README.org's Romacs-vs-GNU tables.

Usage: readme.py RESULTS.json [README.org]

Replaces the text between the `# perfbench-table-begin' and
`# perfbench-table-end' lines of README.org with an Org table of the
editor workloads built from RESULTS.json (perfbench's output), and the
text between `# perfbench-elb-begin' and `# perfbench-elb-end' with one
of GNU's elisp-benchmarks rows, each headed by the build, the GNU pin,
the machine and the run's date.  Every row is kept, losses included.  A
block whose rows the run did not measure is left as it was.
"""
import json
import os
import re
import sys
from pathlib import Path

BLOCKS = {'table': ('# perfbench-table-begin', '# perfbench-table-end'),
          'elb': ('# perfbench-elb-begin', '# perfbench-elb-end')}


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


def public(note):
    """NOTE for a public page: paths lose their directories, and a note
    a refused wall left empty loses its separator."""
    return re.sub(r'(?<![\w.])(?:/[^/\s:\'"]+)+/', '', note).rstrip('; ')


def ratio(value):
    return '-' if value is None else f'{value:.2f}x'


def machine():
    cpu = next((line.split(':', 1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines()
                if line.startswith('model name')), 'unknown CPU')
    mem = next((int(line.split()[1]) for line in Path('/proc/meminfo').read_text().splitlines()
                if line.startswith('MemTotal')), 0)
    return f'{cpu}, {os.cpu_count()} threads, {round(mem / 1048576)} GB'


def measured(meta, what):
    started = meta['started']
    date = f'{started[0:4]}-{started[4:6]}-{started[6:8]}'
    return (f'Measured {date} on build ~{meta["commit"][:8]}~ against pinned GNU Emacs '
            f'~{meta["fixtures"]["commit"][:8]}~, on {machine()}.  '
            f'Each {what} ran {meta["runs"]} times per engine in private home directories; '
            'wall time is the median, and wall times measured above load '
            f'{meta["max_load"]:g} are left out.')


def section(results):
    lines = [
        measured(results['meta'], 'workload') + '  The ratio is Romacs over GNU: below 1.00x, Romacs is faster.',
        '',
        '| Workload | Per | GNU | Romacs | Romacs/GNU (time) | Romacs/GNU (instructions) | Notes |',
        '|-',
    ]
    for r in results['rows']:
        if r['group'] == 'elb':
            continue
        gnu, romacs = r['gnu'] or {}, r['romacs'] or {}
        lines.append(f'| {r["row"]} | {r["unit"]} | {scaled(gnu.get("wall"), "s")} | '
                     f'{scaled(romacs.get("wall"), "s")} | {ratio(r["wall_ratio"])} | '
                     f'{ratio(r["instructions_ratio"])} | {public(r["note"])} |')
    skipped = [line.split(':', 1)[0] for line in results['skipped'] if not line.startswith('elb')]
    if skipped:
        lines += ['', f'Not measured in this run: {", ".join(skipped)}.']
    return '\n'.join(lines)


def elb_section(results):
    """GNU's elisp-benchmarks, a member a row, and their geometric mean."""
    meta = results['meta']
    native = meta.get('elb_native')
    lines = [
        "This is GNU's own benchmark suite, GNU ELPA's elisp-benchmarks (the one GNU uses to evaluate "
        'native compilation), not ours, and it measures pure Lisp compute, not editor responsiveness.',
        '',
        measured(meta, 'member') + '  GNU loads the member files byte-compiled, with JIT native compilation off, '
        + ('and natively compiled at the suite\'s own speed and safety; its own Lisp libraries are '
           'natively compiled in both columns, as this GNU is built with native compilation.  '
           if native else 'and has no native-compilation column: this GNU is built without it.  ')
        + 'Romacs loads their source.  A member an engine does not define, or whose run reported an error, '
        'has no figure there, and the geometric mean leaves it out.  The ratio is Romacs over GNU: '
        'below 1.00x, Romacs is faster.',
        '',
        '| Member | GNU .elc | ' + ('GNU native | ' if native else '') + 'Romacs | Romacs/GNU .elc (time) | '
        + ('Romacs/GNU native (time) | ' if native else '') + 'Romacs/GNU .elc (instructions) | '
        + ('Romacs/GNU native (instructions) | ' if native else '') + 'Notes |',
        '|-',
    ]
    for r in results['rows']:
        if r['group'] != 'elb':
            continue
        gnu, romacs = r['gnu'] or {}, r['romacs'] or {}
        cells = [r['row'].removeprefix('elb '), scaled(gnu.get('wall'), 's')]
        cells += [scaled((r.get('gnu_native') or {}).get('wall'), 's')] if native else []
        cells += [scaled(romacs.get('wall'), 's'), ratio(r['wall_ratio'])]
        cells += [ratio(r.get('native_wall_ratio'))] if native else []
        cells += [ratio(r['instructions_ratio'])]
        cells += [ratio(r.get('native_instructions_ratio'))] if native else []
        cells += [public(r['note'])]
        lines.append('| ' + ' | '.join(cells) + ' |')
    return '\n'.join(lines)


def replace(text, block, body):
    begin, end = BLOCKS[block]
    pattern = re.compile(rf'^{re.escape(begin)}\n.*?^{re.escape(end)}$', re.S | re.M)
    if not pattern.search(text):
        raise SystemExit(f'README: no {begin} ... {end} block')
    return pattern.sub(lambda _: f'{begin}\n{body}\n{end}', text)


def main(argv):
    if not 1 <= len(argv) <= 2:
        raise SystemExit(__doc__)
    results = json.loads(Path(argv[0]).read_text())
    readme = Path(argv[1] if len(argv) == 2 else 'README.org')
    text = readme.read_text()
    groups = {r['group'] for r in results['rows']}
    for block, measured_here, body in (('table', bool(groups - {'elb'}), section),
                                       ('elb', 'elb' in groups, elb_section)):
        if measured_here:
            text = replace(text, block, body(results))
        else:
            print(f'{readme}: this run has no {block} rows; that block is unchanged', file=sys.stderr)
    readme.write_text(text)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
