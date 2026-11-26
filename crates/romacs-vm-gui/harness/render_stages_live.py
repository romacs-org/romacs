#!/usr/bin/env python3
"""Render stages: what one frame costs the render thread, by scenario, held to bars.

Each painting scenario of display_stages.el and render_stages.el runs as
display_stages_live.py runs it: a fresh `BINARY -Q --loadup' window in a
private headless labwc, STEPS steps, each a timer that makes the scenario's
change followed by the loop's own redisplay, which sends the render thread a
scene.  Here the session runs under ROMACS_DIAGNOSTICS=1, so the render
thread writes a `frame-cost' line for every frame it draws (the display
runtime's `frame_cost'): what it added to each of its counts since the frame
before.  A scenario's frames are the lines from its steps' start mark on.

A scene frame is a frame that reached the surface with a new scene in it;
any other frame that reached it is an extra (an animation's, the window
system's).  A scenario's figures are medians over its scene frames (the
upper of two middle ones):

  rows_tessellated  text rows the root frame's glyph passes tessellated
  rows_reused       rows of the frame kept from an earlier frame's pixels
  cursor_rows       rows tessellated to draw a cursor over the root picture
  glyphs_walked     glyph records handed to glyph passes
  font_identities   font identities hashed, one per glyph looked up
  bytes_uploaded    bytes written to vertex buffers and atlas pages
  own_instructions  the thread's instructions in the scene's ingestion and
                    the frame's draw, less those inside the surface's
                    acquire, the queue's submit and the present

and one count over all its frames:

  extra_frames      the frames drawn beside the scene frames, per ten steps

The report has, without bars, `driver_instructions' (acquire, submit and
present: wgpu's and the driver's, which on a software adapter is the
rasterizer) and the extras' median `extra_own_instructions'.

The bars file maps each scenario to a bar per figure.  Costs have ceilings,
reused rows have a floor, and cursor rows are held exactly.  A measured
figure with no bar is reported as "no bar yet".  A bar without its
measurement fails.  --write-bars FILE writes the rows as measured and the
costs a tenth higher.  It requires branch beta or FORGE_TRUNK_JOB=1.

A measurement is refused when the thread has no instruction counter, or
when fewer than nine tenths of the steps drew a scene frame.

The compositor renders on the host's GPU when ROMACS_HARNESS_WLR_ENV gives
it one, and Romacs then draws on that GPU too; the report says which.
--record profiles each scenario's steps as display_stages_live.py does.

Writes OUTPUT/report.json (status PASS or FAIL) and OUTPUT/table.md, and
prints the table.

usage: render_stages_live.py --binary PATH --output DIR [--only NAME ...]
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

import display_stages_live as stages
import scaling_live as scaling
from scaling_live import Refused

HERE = Path(__file__).resolve().parent
SCENARIOS = (stages.SCENARIOS, HERE / 'render_stages.el')
BARS = HERE / 'render_stages_bars.json'
ROWS = ('rows_tessellated', 'rows_reused', 'cursor_rows')
COUNTS = (*ROWS, 'glyphs_walked', 'font_identities', 'bytes_uploaded')
DRIVER = ('acquire_instructions', 'submit_instructions', 'present_instructions')
MARGIN_PERCENT = 110
SCENES = 0.9


# Pure parts: frames, figures, bars and the table.

def frames(stderr_text):
    """The `frame-cost' lines of STDERR_TEXT that reached the surface, from
    the last steps' start mark on: a session's stderr is appended to, so an
    earlier session's frames are never this one's."""
    out, started = [], False
    for line in stderr_text.splitlines():
        if not line.startswith('{'):
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get('stages') == 'start':
            out, started = [], True
        elif started and record.get('diagnostic') == 'frame-cost' and record.get('result') == 'Submitted':
            out.append(record)
    return out


def own(frame):
    """FRAME's instructions in this code: its scene's ingestion and its
    draw, less the surface's and the driver's part of the draw."""
    return frame['ingest_instructions'] + frame['frame_instructions'] - driver(frame)


def driver(frame):
    """FRAME's instructions inside the surface's acquire, the queue's
    submit and the present."""
    return sum(frame[name] for name in DRIVER)


def figures(drawn, steps):
    """A scenario's figures from DRAWN, its frames over STEPS steps, and
    what the report says beside them."""
    scenes = [f for f in drawn if f['scenes']]
    extras = [f for f in drawn if not f['scenes']]
    held = {name: stages.quantile([f[name] for f in scenes], .5) for name in COUNTS}
    held['own_instructions'] = stages.quantile([own(f) for f in scenes], .5)
    held['extra_frames'] = -(-len(extras) * 10 // steps)
    said = {'scene_frames': len(scenes), 'driver_instructions': stages.quantile([driver(f) for f in scenes], .5),
            'extra_own_instructions': stages.quantile([own(f) for f in extras], .5) if extras else 0}
    return held, said


def refusal(drawn, steps):
    """Why DRAWN is no measurement of a scenario over STEPS steps; None
    when it is one."""
    scenes = [f for f in drawn if f['scenes']]
    if len(scenes) < SCENES * steps:
        return f'{len(scenes)} scene frames for {steps} steps'
    if not any(f['frame_instructions'] for f in scenes):
        return 'no instruction counter on the render thread'
    return None


def verdict(scenario, stderr_text, steps, bars):
    """SCENARIO's row from its session's STDERR_TEXT: PASS, or FAIL with a reason."""
    row = {'name': scenario['name']}
    drawn = frames(stderr_text)
    problem = refusal(drawn, steps)
    if problem:
        return {**row, 'status': 'FAIL', 'reason': problem}
    row['figures'], row['said'] = figures(drawn, steps)
    row['frames'] = drawn
    return stages.bar_verdict(row, bars.get(scenario['name'], {}), floors=('rows_reused',), exact=('cursor_rows',))


def ceiling(figure, value):
    """The bar --write-bars sets for FIGURE measured at VALUE: a row count
    as it is, a cost its margin higher, to three digits."""
    if figure in ROWS or value <= 0:
        return value
    raised = -(-value * MARGIN_PERCENT // 100)
    unit = 10 ** max(0, len(str(raised)) - 3)
    return -(-raised // unit) * unit


def bars_of(rows):
    """The bars ROWS' figures would pass."""
    return {row['name']: {figure: ceiling(figure, value) for figure, value in row['figures'].items()}
            for row in rows if 'figures' in row}


def table(rows):
    """The report as a Markdown table, instructions in millions."""
    lines = ['| scenario | tessellated | reused | cursor | glyphs | identities | uploaded | own | driver | extra frames | extra own | status |',
             '|---|---|---|---|---|---|---|---|---|---|---|---|']
    for row in rows:
        f, said = row.get('figures'), row.get('said')
        millions = lambda count: f'{count / 1e6:.2f}M'  # noqa: E731
        cells = ([f['rows_tessellated'], f['rows_reused'], f['cursor_rows'], f['glyphs_walked'], f['font_identities'],
                  f"{f['bytes_uploaded'] / 1024:.0f}K", millions(f['own_instructions']), millions(said['driver_instructions']),
                  f"{f['extra_frames'] / 10:.1f}/step", millions(said['extra_own_instructions'])]
                 if f else ['-'] * 10)
        note = f" — {row['reason']}" if row.get('reason') else ''
        lines.append(f"| {row['name']} | {' | '.join(map(str, cells))} | {row['status']}{note} |")
    return '\n'.join(lines) + '\n' + stages.unbarred_lines(rows)


# Sessions.

def scenarios(binary, home, timeout):
    """The painting scenarios display_stages.el and render_stages.el define."""
    listed = scaling.batch(binary, home, '(display-stages-list)', timeout, loads=SCENARIOS)
    return [{'name': raw['name']} for raw in listed if raw.get('paints') is True]


def measure(gui, scenario, args):
    """SCENARIO's session's stderr, its frames' costs in it; profiled under --record."""
    name = scenario['name']
    prefix, env, data = stages.recording(args.output, gui.home, name) if args.record else ((), {}, None)
    # The session appends to its stderr file: its own lines are those after
    # the file's end as it stands now.
    log = gui.home / f'{name}.stderr'
    start = log.stat().st_size if log.exists() else 0
    gui.session(name, lambda out: f'(display-stages-start (quote {name}) "{out}" {args.steps})',
                loads=SCENARIOS, env={'ROMACS_DIAGNOSTICS': '1', **env}, prefix=prefix, wait=args.timeout)
    if data:
        stages.callgraph(data, args.timeout)
    return since(log.read_bytes(), start)


def since(data, start):
    """The text of DATA, a session's stderr file, from byte START on."""
    return data[start:].decode(errors='replace')


def run(binary, home, selected, bars, args):
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/1000'))
    rows = []
    with tempfile.TemporaryDirectory(prefix='render-', dir=runtime) as temporary:
        Path(temporary).chmod(0o700)
        gui = scaling.Gui(binary, home, Path(temporary), args.timeout)
        try:
            for scenario in selected:
                try:
                    rows.append(verdict(scenario, measure(gui, scenario, args), args.steps, bars))
                except (Refused, RuntimeError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
                    rows.append({'name': scenario['name'], 'status': 'FAIL', 'reason': str(error)})
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
    if args.write_bars and not stages.bar_writer_allowed(HERE):
        parser.error('--write-bars requires beta or FORGE_TRUNK_JOB=1')
    if args.steps < 1:
        parser.error('--steps: at least one step')
    args.output = args.output.resolve()
    args.output.mkdir(parents=True)
    binary = args.binary.resolve()
    home = args.output / 'home'
    report = {'binary': str(binary), 'bars': str(args.bars), 'steps': args.steps, 'status': 'FAIL', 'rows': [],
              'compositor_renderer': scaling.compositor_env().get('WLR_RENDERER', 'pixman')}
    started = time.monotonic()
    try:
        bars = stages.read_bars(args.bars, args.write_bars is not None)
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
        report['status'] = stages.overall(report['rows'])
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
    print(f"{report['status']} on {report['compositor_renderer']} in {report['seconds']}s{': ' + report['error'] if 'error' in report else ''}")
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
