#!/usr/bin/env python3
"""Display stages: what one redisplay costs, by scenario, held to bars.

Each scenario of display_stages.el runs in a fresh `BINARY -Q --loadup'
window in a private headless labwc (scaling_live.py's sessions): a buffer
shown one way, then STEPS steps, each a timer that makes the scenario's
change followed by the loop's own redisplay.  The session reads the
evaluator's instruction counter and `romacs--display-stats' at the start of
every step and after the last one's redisplay, all before it serializes
anything, so a step's figures are differences of two readings:

  step_instructions     everything the step ran: its timer, its change and
                        its redisplay
  display_instructions  the part inside redisplay (the frame frozen and
                        compared, and laid out and submitted if it changed)
  display_conses        conses made inside redisplay
  windows_rebuilt       windows redisplay built in full
  band_expansions       mode lines, header lines and status bars expanded
  layouts               layouts asked of the layout host besides a paint's
                        own: redisplay's, and any the step's own Lisp asked
  frames_submitted      frames sent to the renderer

A scenario's figure is the median over its steps (the upper of two middle
ones); redisplay's instructions have their 90th percentile as well
(`display_instructions_p90': the streamed deltas that add a row).  A step is
odd when it broke its scenario's rule: one redisplay a step, with a frame
submitted in every step of a scenario that changes what the frame shows,
and in none of one that does not.  The host redisplays for reasons of its
own now and then, so the figures are taken over the steps that kept the
rule, and the report names the odd ones.  A measurement is refused when
the thread has no instruction counter, or when more than a tenth of the
steps are odd.  Each step's counts, `collections' among them, are in the
report.

The bars file maps each scenario to a bar per figure.  Instructions and
conses are held under theirs, a ceiling; the windows rebuilt, the bands,
the layouts and the frames are held to theirs exactly, so a scenario whose
step no longer does its change fails as one that costs more does, and work
that removes cost lowers its bars in the same change.  A scenario or figure
without a bar fails.  --write-bars FILE writes the bars this run would
pass: the counts as counted, instructions and conses a tenth higher.

--record profiles each scenario's steps with `perf record', enabled by the
session at its first step and disabled after its last reading, and writes
NAME.perf.data and its top inclusive symbols (NAME.callgraph.txt).  Whole
stacks need a build made with RUSTFLAGS='-C force-frame-pointers=yes'.

Writes OUTPUT/report.json (status PASS or FAIL) and OUTPUT/table.md, and
prints the table.

usage: display_stages_live.py --binary PATH --output DIR [--only NAME ...]
                              [--bars FILE] [--write-bars FILE] [--steps 30]
                              [--record] [--timeout 300]
"""
import argparse
import json
import math
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import scaling_live as scaling
from scaling_live import Refused

HERE = Path(__file__).resolve().parent
SCENARIOS = HERE / 'display_stages.el'
BARS = HERE / 'display_stages_bars.json'
# A figure and the count of a reading it is the difference of.
COUNTS = {'step_instructions': 'step-instructions', 'display_instructions': 'instructions', 'display_conses': 'conses',
          'windows_rebuilt': 'windows-rebuilt', 'band_expansions': 'band-expansions', 'layouts': 'layouts',
          'frames_submitted': 'frames-submitted'}
# The figures whose 90th percentile is held too (a whole step's varies
# with the timers that poll for its redisplay), the figures held under a
# ceiling and the margin --write-bars gives them, and the share of steps
# that may be odd.
PEAKS = ('display_instructions',)
MARGINED = ('instructions', 'conses')
MARGIN = 1.10
ODD = 0.1


# Pure parts: steps, figures, bars and the table.

def steps(readings):
    """Each step's counts: the differences of consecutive READINGS."""
    return [{name: after[name] - before[name] for name in after} for before, after in zip(readings, readings[1:])]


def quantile(values, share):
    """The value SHARE of the way through VALUES in order: the upper of the
    two middle ones at a half."""
    return sorted(values)[min(len(values) - 1, int(len(values) * share))]


def figures(deltas):
    """A scenario's figures from its steps' counts."""
    out = {figure: quantile([d[count] for d in deltas], .5) for figure, count in COUNTS.items()}
    out.update({f'{figure}_p90': quantile([d[COUNTS[figure]] for d in deltas], .9) for figure in PEAKS})
    return out


def odd_steps(scenario, deltas):
    """The steps that broke SCENARIO's rule: one redisplay, with a frame
    submitted in every step of one that paints, and in none of one that
    does not."""
    return [i for i, d in enumerate(deltas) if d['cycles'] != 1 or (d['frames-submitted'] == 0) == scenario['paints']]


def refusal(scenario, result, wanted):
    """Why RESULT, a session's readings, is no measurement of SCENARIO over
    WANTED steps; None when it is one."""
    readings = result.get('readings') or []
    if len(readings) != wanted + 1:
        return f'{len(readings)} readings for {wanted} steps'
    if any(r.get('step-instructions') is None or r.get('instructions') is None for r in readings):
        return 'no instruction counter (romacs-instructions is nil)'
    odd = odd_steps(scenario, steps(readings))
    if len(odd) > ODD * wanted:
        return (f"steps {odd} did not run one redisplay that submitted "
                f"{'a frame, as a step that changes the frame does' if scenario['paints'] else 'no frame, as a step that changes nothing shown does'}")
    return None


def off(measured, bars):
    """The figures of MEASURED off their bar or without one: (figure,
    value, bar or None).  Instructions and conses are off over their bar,
    any other count when it differs."""
    return [(figure, value, bars.get(figure)) for figure, value in measured.items()
            if bars.get(figure) is None or value > bars[figure] or (value < bars[figure] and not margined(figure))]


def verdict(scenario, result, wanted, bars):
    """SCENARIO's row from its session's RESULT: PASS, or FAIL with a reason."""
    row = {'name': scenario['name'], 'paints': scenario['paints']}
    problem = refusal(scenario, result, wanted)
    if problem:
        return {**row, 'status': 'FAIL', 'reason': problem}
    deltas = steps(result['readings'])
    odd = odd_steps(scenario, deltas)
    kept = [d for i, d in enumerate(deltas) if i not in odd]
    row.update(chars=result.get('chars'), figures=figures(kept), odd=odd, steps=deltas)
    if scenario['name'] not in bars:
        return {**row, 'status': 'FAIL', 'reason': 'no bars for this scenario'}
    row['off'] = off(row['figures'], bars[scenario['name']])
    if row['off']:
        reason = '; '.join(f'{figure} {value}, ' + ('no bar' if bar is None else f"{'over' if value > bar else 'under'} {bar}")
                           for figure, value, bar in row['off'])
        return {**row, 'status': 'FAIL', 'reason': reason}
    return {**row, 'status': 'PASS'}


def margined(figure):
    """Whether FIGURE is held under a ceiling with a margin, not exactly."""
    return any(kind in figure for kind in MARGINED)


def ceiling(figure, value):
    """The bar --write-bars sets for FIGURE measured at VALUE: instructions
    and conses their margin higher, to three digits, any other count as
    it is."""
    if not margined(figure) or value <= 0:
        return value
    raised = value * MARGIN
    unit = 10 ** max(0, math.floor(math.log10(raised)) - 2)
    return math.ceil(raised / unit) * unit


def bars_of(rows):
    """The bars ROWS' figures would pass."""
    return {row['name']: {figure: ceiling(figure, value) for figure, value in row['figures'].items()}
            for row in rows if 'figures' in row}


def overall(rows):
    """PASS unless a row failed or there are none."""
    return 'PASS' if rows and all(row['status'] == 'PASS' for row in rows) else 'FAIL'


def table(rows):
    """The report as a Markdown table, instructions in millions."""
    lines = ['| scenario | step | display | display p90 | conses | rebuilt | bands | layouts | frames | status |',
             '|---|---|---|---|---|---|---|---|---|---|']
    for row in rows:
        f = row.get('figures')
        millions = lambda name: f'{f[name] / 1e6:.2f}M'  # noqa: E731
        cells = ([millions('step_instructions'), millions('display_instructions'), millions('display_instructions_p90'),
                  f['display_conses'], f['windows_rebuilt'], f['band_expansions'], f['layouts'], f['frames_submitted']]
                 if f else ['-'] * 8)
        note = f" — {row['reason']}" if row.get('reason') else (f" — odd steps {row['odd']}" if row.get('odd') else '')
        lines.append(f"| {row['name']} | {' | '.join(map(str, cells))} | {row['status']}{note} |")
    return '\n'.join(lines) + '\n'


# Sessions.

def scenarios(binary, home, timeout):
    """The scenarios display_stages.el defines."""
    listed = scaling.batch(binary, home, '(display-stages-list)', timeout, loads=(SCENARIOS,))
    return [{'name': raw['name'], 'paints': raw.get('paints') is True} for raw in listed]


def recording(output, home, name):
    """The `perf record' prefix and environment that profile scenario NAME's
    steps, perf waiting disabled for the session to enable it, and the
    profile's file."""
    data, fifo = output / f'{name}.perf.data', home / 'tmp' / f'{name}.perf-ctl'
    os.mkfifo(fifo)
    prefix = ('perf', 'record', '-q', '-D', '-1', '--control', f'fifo:{fifo}', '-e', 'instructions:u',
              '--call-graph', 'fp', '--user-callchains', '-o', str(data), '--')
    return prefix, {'SCALING_PERF_CTL': str(fifo)}, data


def callgraph(data, timeout):
    """DATA's top inclusive symbols, written beside it."""
    report = subprocess.run(['perf', 'report', '-i', str(data), '--stdio', '--children', '--percent-limit', '3',
                             '--sort', 'symbol', '-g', 'none'], capture_output=True, text=True, timeout=timeout)
    text = '\n'.join(' '.join(line.split()) for line in report.stdout.splitlines()
                     if line.strip() and not line.startswith('#'))
    data.with_name(data.name.replace('.perf.data', '.callgraph.txt')).write_text(text + '\n')


def measure(gui, scenario, args):
    """SCENARIO's readings from a fresh session, profiled under --record."""
    name = scenario['name']
    prefix, env, data = recording(args.output, gui.home, name) if args.record else ((), {}, None)
    # No paint diagnostics: a step costs what it costs a session in use.
    result, _ = gui.session(name, lambda out: f'(display-stages-start (quote {name}) "{out}" {args.steps})',
                            loads=(SCENARIOS,), env={'ROMACS_DIAGNOSTICS': '0', **env}, prefix=prefix, wait=args.timeout)
    if data:
        callgraph(data, args.timeout)
    return result


def run(binary, home, selected, bars, args):
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/1000'))
    rows = []
    with tempfile.TemporaryDirectory(prefix='stages-', dir=runtime) as temporary:
        Path(temporary).chmod(0o700)
        gui = scaling.Gui(binary, home, Path(temporary), args.timeout)
        try:
            for scenario in selected:
                try:
                    rows.append(verdict(scenario, measure(gui, scenario, args), args.steps, bars))
                except (Refused, RuntimeError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
                    rows.append({'name': scenario['name'], 'paints': scenario['paints'], 'status': 'FAIL', 'reason': str(error)})
        finally:
            gui.close()
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--only', nargs='*', default=None)
    parser.add_argument('--bars', type=Path, default=BARS)
    parser.add_argument('--write-bars', type=Path)
    parser.add_argument('--steps', type=int, default=30)
    parser.add_argument('--record', action='store_true')
    parser.add_argument('--timeout', type=float, default=300, help='seconds per session')
    args = parser.parse_args()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True)
    binary = args.binary.resolve()
    home = args.output / 'home'
    report = {'binary': str(binary), 'bars': str(args.bars), 'steps': args.steps, 'status': 'FAIL', 'rows': []}
    started = time.monotonic()
    try:
        bars = json.loads(args.bars.read_text()) if args.bars.is_file() else {}
        scaling.prepare(binary, home, args.timeout)
        selected = scenarios(binary, home, args.timeout)
        if args.only is not None:
            unknown = set(args.only) - {s['name'] for s in selected}
            if unknown:
                raise Refused(f'no such scenarios: {sorted(unknown)}')
            selected = [s for s in selected if s['name'] in args.only]
        report['rows'] = run(binary, home, selected, bars, args)
        report['status'] = overall(report['rows'])
        if args.write_bars:
            args.write_bars.write_text(json.dumps({**bars, **bars_of(report['rows'])}, indent=1) + '\n')
    except (Refused, subprocess.SubprocessError, OSError, ValueError) as error:
        report['error'] = str(error)
    # The worlds are the bulk of HOME, and no part of the report.
    shutil.rmtree(home / 'cache', ignore_errors=True)
    report['seconds'] = round(time.monotonic() - started, 1)
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    text = table(report['rows'])
    (args.output / 'table.md').write_text(text)
    print(text, end='')
    print(f"{report['status']} in {report['seconds']}s{': ' + report['error'] if 'error' in report else ''}")
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
