#!/usr/bin/env python3
"""Display stages: what one redisplay costs, by scenario, held to bars.

Each scenario of display_stages.el runs in a fresh `BINARY -Q --loadup'
window in a private headless labwc (scaling_live.py's sessions): a buffer
shown one way, then STEPS steps, each a timer that makes the scenario's
change followed by the loop's own redisplay and those of what the change
set off: a step ends once it has settled, its redisplay run, no timer ripe
and a redisplay run after any timer that ran (`display-stages--settled-p'),
so where a follow-up redisplay counts does not hang on the host's speed.
The session reads the
evaluator's instruction counter and `romacs--display-stats' at the start of
every step and after the last one's redisplay, all before it serializes
anything, so a step's figures are differences of two readings:

  step_instructions     what Romacs ran for the step: its change, the
                        timers it set off, its redisplays and the
                        redisplay trigger's checks, each once,
                        not the loop's own polls nor the host's wait for a
                        frame to render (`display-stages--work'), nor the
                        host's passes for process output and the echo
                        area's sync, which no scenario sets work for; a
                        step that waits inside its change makes no
                        measurement
  cycles                redisplays the step ran: its own, and any after a
                        timer it set off (jit-lock's `jit-lock-force-redisplay');
                        none for a step that changed nothing shown, which
                        the redisplay trigger finds current
  trigger_instructions  what the redisplay trigger's checks ran
                        (`Vm::display_stale'): whether the frame went stale
  display_instructions  the part inside redisplays (the frame frozen and
                        compared, and laid out and submitted if it changed)
  display_conses        conses made inside redisplay
  windows_rebuilt       windows redisplay built in full
  band_expansions       mode lines, header lines and status bars expanded
  layouts               layouts asked of the layout host besides a paint's
                        own: redisplay's, and any the step's own Lisp asked
  frames_submitted      frames sent to the renderer
  lines_laid            display lines the layout engine laid out, some
                        row of them wrapped (the rest it had kept)
  rows_wrapped          rows the layout engine wrapped
  chars_fontified       characters `jit-lock' asked its functions for, in
                        a scenario that counts them (fontification's)
  above_instructions    what a step's collection ran above the one the
                        session measured before the scenario's setup (its
                        baseline), in a scenario that has one: what the
                        setup added to a full collection

A scenario's figure is the median over its steps (the upper of two middle
ones); redisplay's instructions have their 90th percentile as well
(`display_instructions_p90': the streamed deltas that add a row).  A step is
odd when it broke its scenario's rule: no more redisplays than its steps
usually run (their median), with a frame submitted in every step of a
scenario that changes what the frame shows, and in none of one that does
not.  The host redisplays for reasons of its
own now and then, so the figures are taken over the steps that kept the
rule, and the report names the odd ones; characters fontified, which the
step's change alone decides and an extra redisplay adds nothing to, over
every step: in a typing scenario they grow a character a step, so a
median over the steps kept would move with which were left out.  A measurement is refused when
the thread has no instruction counter, or when more than a tenth of the
steps are odd.  Each step's counts, `collections' among them, are in the
report.

The bars file maps each scenario to ceilings for its figures.  Less work
passes.  A measured figure with no bar is reported as "no bar yet".  A
figure whose bar is "report" is measured for information only.  A bar
without its measurement fails.  --write-bars FILE writes the counts as
measured and the instruction and cons ceilings a tenth higher.  It requires
branch beta or FORGE_TRUNK_JOB=1.

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
COUNTS = {'step_instructions': 'step-instructions', 'cycles': 'cycles', 'display_instructions': 'instructions', 'display_conses': 'conses',
          'windows_rebuilt': 'windows-rebuilt', 'band_expansions': 'band-expansions', 'layouts': 'layouts',
          'frames_submitted': 'frames-submitted', 'lines_laid': 'lines-laid', 'rows_wrapped': 'rows-wrapped',
          'trigger_instructions': 'trigger-instructions'}
# Figures only the scenarios that count them have.
OPTIONAL = {'chars_fontified': 'fontified-chars', 'above_instructions': 'above-instructions'}
# A bar that holds nothing: the figure is reported beside the others.
REPORTED = 'report'
# The figures whose 90th percentile is held too (a redisplay's: the
# streamed deltas that add a row), the figures held under a
# ceiling and the margin --write-bars gives them, and the share of steps
# that may be odd.
PEAKS = ('display_instructions',)
MARGINED = ('instructions', 'conses')
MARGIN_PERCENT = 110
ODD = 0.1


# Pure parts: steps, figures, bars and the table.

def steps(readings):
    """Each step's counts: the differences of consecutive READINGS."""
    return [{name: after[name] - before[name] for name in after} for before, after in zip(readings, readings[1:])]


def quantile(values, share):
    """The value SHARE of the way through VALUES in order: the upper of the
    two middle ones at a half."""
    return sorted(values)[min(len(values) - 1, int(len(values) * share))]


def figures(deltas, every=None):
    """A scenario's figures from its steps' counts, with the optional ones
    its steps counted: those, which the step's change alone decides, over
    EVERY step when given."""
    every = every or deltas
    counted = {**COUNTS, **{figure: count for figure, count in OPTIONAL.items() if all(count in d for d in every)}}
    out = {figure: quantile([d[count] for d in (every if figure in OPTIONAL else deltas)], .5) for figure, count in counted.items()}
    out.update({f'{figure}_p90': quantile([d[COUNTS[figure]] for d in deltas], .9) for figure in PEAKS})
    return out


def odd_steps(scenario, deltas):
    """The steps that broke SCENARIO's rule: no more redisplays than its
    steps usually run, with a frame submitted in every step of one that
    paints, and in none of one that does not.  A step that needed fewer is
    no host's extra: a timer the step set off that left the frame as it
    was runs no redisplay (the redisplay trigger), so where jit-lock's
    forced redisplay finds nothing to redraw a step runs one fewer."""
    usual = quantile([d['cycles'] for d in deltas], .5)
    return [i for i, d in enumerate(deltas) if d['cycles'] > usual or (d['frames-submitted'] == 0) == scenario['paints']]


def refusal(scenario, result, wanted):
    """Why RESULT, a session's readings, is no measurement of SCENARIO over
    WANTED steps; None when it is one."""
    readings = result.get('readings') or []
    if result.get('waited'):
        return f"a step waited, counting its waiting as its work: {result['waited']}"
    if len(readings) != wanted + 1:
        return f'{len(readings)} readings for {wanted} steps'
    if any(r.get('step-instructions') is None or r.get('instructions') is None for r in readings):
        return 'no instruction counter (romacs-instructions is nil)'
    if scenario.get('baseline') and any('above-instructions' not in r for r in readings):
        return 'no baseline was measured: no collection above it is counted'
    if scenario.get('collects') and any(d.get('collected', 0) < 1 for d in steps(readings)):
        return 'a step ran no full collection of its own'
    odd = odd_steps(scenario, steps(readings))
    if len(odd) > ODD * wanted:
        return (f"steps {odd} ran more redisplays than the others, or did not run them with "
                f"{'a frame submitted, as a step that changes the frame does' if scenario['paints'] else 'no frame submitted, as a step that changes nothing shown does'}")
    return None


def bar_verdict(row, bars, *, floors=(), exact=()):
    """Hold ROW's figures to their BARS and report measurements without bars."""
    measured = row['figures']
    off = [(name, value, bar) for name, value in measured.items()
           if (bar := bars.get(name)) is not None and bar != REPORTED
           and (value < bar if name in floors else value > bar or (name in exact and value != bar))]
    off.extend((name, None, bar) for name, bar in bars.items()
               if isinstance(bar, (int, float)) and not isinstance(bar, bool) and name not in measured)
    unbarred = [(name, value) for name, value in measured.items() if bars.get(name) is None]
    row = {**row, 'status': 'FAIL' if off else 'PASS', 'off': off, 'unbarred': unbarred}
    if off:
        row['reason'] = '; '.join(f'{name} not measured, its bar {bar}' if value is None else
                                  f"{name} {value}, {'over' if value > bar else 'under'} {bar}"
                                  for name, value, bar in off)
    return row


def unbarred_lines(rows):
    """List every figure still awaiting a trunk bar."""
    lines = [f"- {row['name']} · {name} {value}, no bar yet"
             for row in rows for name, value in row.get('unbarred', [])]
    return '\n' + '\n'.join(lines) + '\n' if lines else ''


def verdict(scenario, result, wanted, bars):
    """SCENARIO's row from its session's RESULT: PASS, or FAIL with a reason."""
    row = {'name': scenario['name'], 'paints': scenario['paints'], 'session': result.get('session')}
    problem = refusal(scenario, result, wanted)
    if problem:
        return {**row, 'status': 'FAIL', 'reason': problem}
    deltas = steps(result['readings'])
    odd = odd_steps(scenario, deltas)
    kept = [d for i, d in enumerate(deltas) if i not in odd]
    row.update(chars=result.get('chars'), figures=figures(kept, deltas), odd=odd, steps=deltas)
    return bar_verdict(row, bars.get(scenario['name'], {}))


def rebarred(bars, new):
    """BARS with each scenario's figures from NEW, its notes (FIGURE_note)
    kept: why a bar is what it is outlives a recording."""
    kept = lambda name: {k: v for k, v in bars.get(name, {}).items() if k.endswith('_note') or v == REPORTED}
    return {**bars, **{name: {**figures, **kept(name)} for name, figures in new.items()}}


def margined(figure):
    """Whether FIGURE gets a margin when recording its ceiling."""
    return any(kind in figure for kind in MARGINED)


def ceiling(figure, value):
    """The bar --write-bars sets for FIGURE measured at VALUE: instructions
    and conses their margin higher, to three digits, any other count as
    it is."""
    if not margined(figure) or value <= 0:
        return value
    raised = -(-value * MARGIN_PERCENT // 100)
    unit = 10 ** max(0, len(str(raised)) - 3)
    return -(-raised // unit) * unit


def bars_of(rows):
    """The bars ROWS' figures would pass."""
    return {row['name']: {figure: ceiling(figure, value) for figure, value in row['figures'].items()}
            for row in rows if 'figures' in row}


def overall(rows):
    """PASS unless a row failed or there are none."""
    return 'PASS' if rows and all(row['status'] == 'PASS' for row in rows) else 'FAIL'


def table(rows):
    """The report as a Markdown table, instructions in millions."""
    lines = ['| scenario | step | display | display p90 | conses | rebuilt | bands | layouts | frames | lines laid | rows wrapped | status |',
             '|---|---|---|---|---|---|---|---|---|---|---|---|']
    for row in rows:
        f = row.get('figures')
        millions = lambda name: f'{f[name] / 1e6:.2f}M'  # noqa: E731
        cells = ([millions('step_instructions'), millions('display_instructions'), millions('display_instructions_p90'),
                  f['display_conses'], f['windows_rebuilt'], f['band_expansions'], f['layouts'], f['frames_submitted'],
                  f['lines_laid'], f['rows_wrapped']]
                 if f else ['-'] * 10)
        note = f" — {row['reason']}" if row.get('reason') else (f" — odd steps {row['odd']}" if row.get('odd') else '')
        if f and 'above_instructions' in f:
            note += f" · {millions('above_instructions')} above its baseline"
        if row.get('session') and row['name'].endswith('-collected'):
            note += f" · {row['session']['symbols']} symbols, {row['session']['files']} files"
        lines.append(f"| {row['name']} | {' | '.join(map(str, cells))} | {row['status']}{note} |")
    return '\n'.join(lines) + '\n' + unbarred_lines(rows)


# Sessions.

def scenarios(binary, home, timeout):
    """The scenarios display_stages.el defines."""
    listed = scaling.batch(binary, home, '(display-stages-list)', timeout, loads=(SCENARIOS,))
    return [{'name': raw['name'], 'paints': raw.get('paints') is True,
             'collects': raw.get('collects') is True, 'baseline': raw.get('baseline') is True} for raw in listed]


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


def bar_writer_allowed(root):
    """Allow recording from the source checkout's beta branch or a trunk job."""
    if os.environ.get('FORGE_TRUNK_JOB') == '1':
        return True
    env = {key: value for key, value in os.environ.items()
           if key not in ('GIT_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE')}
    try:
        branch = subprocess.run(['git', '-C', str(root), 'symbolic-ref', '--quiet', '--short', 'HEAD'],
                                env=env, capture_output=True, text=True, check=False)
    except OSError:
        return False
    return branch.returncode == 0 and branch.stdout.strip() == 'beta'


def read_bars(path, writing):
    """Read the bars at PATH, allowing a writer to start a new file."""
    if writing and not path.exists():
        return {}
    return json.loads(path.read_text())


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
    if args.write_bars and not bar_writer_allowed(HERE):
        parser.error('--write-bars requires beta or FORGE_TRUNK_JOB=1')
    if args.steps < 1:
        parser.error('--steps: at least one step')
    args.output = args.output.resolve()
    args.output.mkdir(parents=True)
    binary = args.binary.resolve()
    home = args.output / 'home'
    report = {'binary': str(binary), 'bars': str(args.bars), 'steps': args.steps, 'status': 'FAIL', 'rows': []}
    started = time.monotonic()
    try:
        bars = read_bars(args.bars, args.write_bars is not None)
        scaling.prepare(binary, home, args.timeout)
        selected = scenarios(binary, home, args.timeout)
        stale = sorted(set(bars) - {s['name'] for s in selected})
        if stale:
            raise Refused(f'bars for no scenario: {stale}')
        if args.only is not None:
            unknown = set(args.only) - {s['name'] for s in selected}
            if unknown:
                raise Refused(f'no such scenarios: {sorted(unknown)}')
            selected = [s for s in selected if s['name'] in args.only]
        report['rows'] = run(binary, home, selected, bars, args)
        report['status'] = overall(report['rows'])
        if args.write_bars:
            args.write_bars.write_text(json.dumps(rebarred(bars, bars_of(report['rows'])), indent=1) + '\n')
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
