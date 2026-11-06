#!/usr/bin/env python3
"""Write a perfbench run's results into README.org's Romacs comparison tables.

Usage: readme.py RESULTS.json [README.org]

Replaces the text between the `# perfbench-table-begin' and
`# perfbench-table-end' lines of README.org with an Org table of the
editor workloads built from RESULTS.json (perfbench's output), and the
text between `# perfbench-elb-begin' and `# perfbench-elb-end' with one
of GNU's elisp-benchmarks rows, each headed by the build, the GNU pin,
the machine and the run's date.  Every row is kept, losses included.  A
block whose rows the run did not measure is left as it was.  Between
`# perfbench-headlines-begin' and `# perfbench-headlines-end' it lists the
rows where Romacs clearly beats a rival, with build and rerun command, and
names on standard error any headline this run no longer has.
"""
import json
import os
import re
import sys
from pathlib import Path

BLOCKS = {'table': ('# perfbench-table-begin', '# perfbench-table-end'),
          'headlines': ('# perfbench-headlines-begin', '# perfbench-headlines-end'),
          'elb': ('# perfbench-elb-begin', '# perfbench-elb-end')}
# A headline: this ratio or better on every metric the run measured.
MARGIN = .80
# (rival, its column, its time ratio, its instructions ratio)
RIVALS = (('GNU', 'gnu', 'wall_ratio', 'instructions_ratio'),
          ('GNU native', 'gnu_native', 'native_wall_ratio', 'native_instructions_ratio'),
          ('neomacs', 'neomacs', 'neomacs_wall_ratio', 'neomacs_instructions_ratio'))
SUMMARIES = ('elb geometric mean', 'elb median')


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


def installation(meta):
    """How this run's Romacs ran, for a table's header."""
    install = meta.get('install')
    if not install:
        return ('Romacs read its Lisp from source, with no compiled store or batch world: '
                'not how an installation runs.')
    return (f'Romacs ran as an installation does: {install["files"] - install["not_compiled"]} of the '
            f'{install["files"]} files of its Lisp compiled ahead of use into its store by '
            '=--compile-lisp-tree=, before anything was timed; batch rows booted from a batch world prepared first, '
            "which a user's first batch run builds, and a workload's own file loaded compiled.  Each row's note "
            'counts the loads the store confirms and names any it does not.')


def rival(meta):
    """How this run's neomacs ran, for a table's header; empty without one."""
    build = meta.get('neomacs')
    if not build:
        return ''
    commit = f', commit ~{build["commit"][:8]}~' if build.get('commit') else ''
    return (f'  {build["version"]}{commit} ran as it installs: its dump and its =.elc= files, a workload\'s own file '
            'byte-compiled by its own compiler.  A row it cannot run says why.')


def section(results):
    neomacs = results['meta'].get('neomacs')
    lines = [
        measured(results['meta'], 'workload') + '  ' + installation(results['meta']) + rival(results['meta'])
        + '  A ratio is Romacs over the other engine: below 1.00x, Romacs is faster.',
        '',
        '| Workload | Per | GNU | Romacs | ' + ('neomacs | ' if neomacs else '')
        + 'Romacs/GNU (time) | Romacs/GNU (instructions) | '
        + ('Romacs/neomacs (time) | Romacs/neomacs (instructions) | ' if neomacs else '') + 'Notes |',
        '|-',
    ]
    for r in results['rows']:
        if r['group'] == 'elb':
            continue
        gnu, romacs = r['gnu'] or {}, r['romacs'] or {}
        cells = [r['row'], r['unit'], scaled(gnu.get('wall'), 's'), scaled(romacs.get('wall'), 's')]
        cells += [scaled((r.get('neomacs') or {}).get('wall'), 's')] if neomacs else []
        cells += [ratio(r['wall_ratio']), ratio(r['instructions_ratio'])]
        cells += [ratio(r.get('neomacs_wall_ratio')), ratio(r.get('neomacs_instructions_ratio'))] if neomacs else []
        lines.append('| ' + ' | '.join(cells + [public(r['note'])]) + ' |')
    skipped = [line.split(':', 1)[0] for line in results['skipped'] if not line.startswith('elb')]
    if skipped:
        lines += ['', f'Not measured in this run: {", ".join(skipped)}.']
    return '\n'.join(lines)


def elb_section(results):
    """GNU's elisp-benchmarks, a member a row, and their geometric mean."""
    meta = results['meta']
    native, neomacs = meta.get('elb_native'), meta.get('neomacs')
    lines = [
        "This is GNU's own benchmark suite, GNU ELPA's elisp-benchmarks (the one GNU uses to evaluate "
        'native compilation), not ours, and it measures pure Lisp compute, not editor responsiveness.',
        '',
        measured(meta, 'member') + '  GNU loads the member files byte-compiled, with JIT native compilation off, '
        + ('and natively compiled at the suite\'s own speed and safety; its own Lisp libraries are '
           'natively compiled in both columns, as this GNU is built with native compilation.  '
           if native else 'and has no native-compilation column: this GNU is built without it.  ')
        + installation(meta) + rival(meta) + '  A member an engine does not define, or whose run reported an error, '
        'has no figure there.  A ratio is Romacs over the other engine: below 1.00x, Romacs is faster.  '
        'The geometric mean compares Romacs with GNU =.elc= alone.  Every ratio also has a median over the members '
        'that have it, and the median is the summary to read against native GNU: native compilation reduces some '
        'members to almost nothing, and one such ratio would decide a mean.',
        '',
        '| Member | GNU .elc | ' + ('GNU native | ' if native else '') + 'Romacs | ' + ('neomacs | ' if neomacs else '')
        + 'Romacs/GNU .elc (time) | ' + ('Romacs/GNU native (time) | ' if native else '')
        + 'Romacs/GNU .elc (instructions) | ' + ('Romacs/GNU native (instructions) | ' if native else '')
        + ('Romacs/neomacs (time) | Romacs/neomacs (instructions) | ' if neomacs else '') + 'Notes |',
        '|-',
    ]
    for r in results['rows']:
        if r['group'] != 'elb':
            continue
        gnu, romacs = r['gnu'] or {}, r['romacs'] or {}
        cells = [r['row'].removeprefix('elb '), scaled(gnu.get('wall'), 's')]
        cells += [scaled((r.get('gnu_native') or {}).get('wall'), 's')] if native else []
        cells += [scaled(romacs.get('wall'), 's')]
        cells += [scaled((r.get('neomacs') or {}).get('wall'), 's')] if neomacs else []
        cells += [ratio(r['wall_ratio'])]
        cells += [ratio(r.get('native_wall_ratio'))] if native else []
        cells += [ratio(r['instructions_ratio'])]
        cells += [ratio(r.get('native_instructions_ratio'))] if native else []
        cells += [ratio(r.get('neomacs_wall_ratio')), ratio(r.get('neomacs_instructions_ratio'))] if neomacs else []
        cells += [public(r['note'])]
        lines.append('| ' + ' | '.join(cells) + ' |')
    return '\n'.join(lines)


def selector(made):
    """The --only that reruns the row MADE."""
    if made['group'] == 'gui':
        return 'launch' if 'launch' in made['row'] or 'world' in made['row'] else 'keys'
    return {'micro': 'micro', 'elb': 'elb', 'ert': 'ERT 25 files', 'init': 'init', 'apps': 'apps'}.get(made['group'], made['row'])


def wins(results):
    """(row, rival, instructions ratio, time ratio) of every row where Romacs
    beats a rival by MARGIN on every metric measured.  A summary is no row.
    A figure at or below zero on either side is a count lost in its setup or
    idle floor, and two of them can make a fine ratio, so it wins nothing."""
    for made in results['rows']:
        if made['row'] in SUMMARIES:
            continue
        for name, column, time, instructions in RIVALS:
            figures = [(made.get(c) or {}).get(metric) for c in (column, 'romacs') for metric in ('instructions', 'wall')]
            ratios = [made.get(instructions), made.get(time)]
            measured_ = [value for value in ratios if value is not None]
            if measured_ and max(measured_) <= MARGIN and all(f is None or f > 0 for f in figures):
                yield made, name, ratios[0], ratios[1]


def headlines(results):
    """The wins as a table: a row stays in the main tables whatever this says."""
    meta = results['meta']
    lines = [f'Every row of this run where Romacs beats a rival by at least {round((1 - MARGIN) * 100)}%: '
             f'{MARGIN:.2f}x or better on every metric the run measured.  Build ~{meta["commit"][:8]}~.  '
             'The tables keep every row, losses included.', '',
             '| Row | Against | Instructions | Time | Build | Rerun |', '|-']
    for made, name, instructions, time in wins(results):
        third = ' --neomacs NEOMACS' if name == 'neomacs' else ''
        lines.append(f'| {made["row"]} | {name} | {ratio(instructions)} | {ratio(time)} | ~{meta["commit"][:8]}~ | '
                     f"=perfbench.py --romacs BINARY{third} --only '{selector(made)}'= |")
    return '\n'.join(lines)


def kept_wins(text):
    """The (row, rival) pairs TEXT's headlines block lists now."""
    begin, end = BLOCKS['headlines']
    block = text[text.index(begin):text.index(end)] if begin in text else ''
    return {tuple(cell.strip() for cell in line.split('|')[1:3]) for line in block.splitlines()
            if line.startswith('| ') and not line.startswith('| Row ')}


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
    whole = bool(groups - {'elb'}) and 'elb' in groups
    for block, measured_here, body in (('table', bool(groups - {'elb'}), section),
                                       ('elb', 'elb' in groups, elb_section),
                                       ('headlines', whole, headlines)):
        if measured_here:
            if block == 'headlines':
                lost = kept_wins(text) - {(made['row'], name) for made, name, *_ in wins(results)}
                for row_, name in sorted(lost):
                    print(f'{readme}: no longer a headline: {row_} against {name}', file=sys.stderr)
            text = replace(text, block, body(results))
        else:
            print(f'{readme}: this run has no {block} rows; that block is unchanged', file=sys.stderr)
    readme.write_text(text)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
