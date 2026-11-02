#!/usr/bin/env python3
"""Layer animation in a live session: what each animation frame costs, and
that nothing draws once the layers settle.

One private headless labwc and one `romacs-vm-gui --loadup` with
`romacs-complete-mode', `ROMACS_LAYER_ANIMATION=1' (the opt-in a software
renderer needs) and a presentation trace (ROMACS_PRESENT_TRACE).  With
--renderer gles2 the compositor renders on the host GPU (EGL from
--mesa, a Mesa whose libEGL_mesa and GBM the compositor's own libraries
can load), and offers the client dmabuf surfaces, so the client renders
on the GPU too; pixman leaves the client on llvmpipe.  The session runs
these phases, each followed by a pause longer than any animation:

  open     C-x C-f opens the minibuffer's layer (fade and slide)
  type     the keys of --input, each refining the candidate list
  shrink   the keys of --shrink, leaving fewer rows (the rect tweens)
  grow     as many DEL, the rows back
  move     C-n three times (the selection glides)
  close    C-g (fade)
  idle     --idle seconds with nothing to do

With --slowdown N the animations run N times longer, and --shots takes
a screenshot (grim) at each phase's start and a third of the slowed open
length into it, to show the animation's middle.

Reports per phase the draws whose frame plan the child-frame motion
demanded and their durations (the draw's first trace record to its
present call), as p50/p95/max in ms, and the draws during idle.  With
--budget-ms the run fails when an animation frame's p95 exceeds it or
anything draws while idle; with --expect-none it fails when any
animation frame draws (the default, unforced software renderer).

usage: layer_anim_live.py --binary BIN --output DIR [--renderer gles2]
       [--mesa MESA_DIR] [--input file-1] [--idle 3] [--budget-ms 4] [--no-force]
       [--expect-none] [--init FILE] [--slowdown N] [--shots]
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

import live_oracle as live

KEYSYMS = {'-': 'minus', '/': 'slash', ' ': 'space'}
INIT = "(romacs-complete-mode 1)\n"


def wait(pred, timeout, what, proc):
    end = time.time() + timeout
    while time.time() < end:
        if proc is not None and proc.poll() is not None:
            raise RuntimeError(f'binary exited while waiting for {what}')
        if pred():
            return
        time.sleep(.05)
    raise RuntimeError(f'timeout: {what}')


def percentile(values, q):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, round(q * (len(ordered) - 1)))]


def draws(trace):
    """Each draw of the trace: its plan's reasons, first and present-call ns."""
    out = {}
    for line in trace.read_text().split('\n') if trace.exists() else []:
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        seq = record.get('draw_seq')
        if seq is None or record['stage'] == 'presented':
            continue
        draw = out.setdefault(seq, {'start': record['ns'], 'end': None, 'reasons': ''})
        draw['start'] = min(draw['start'], record['ns'])
        draw['reasons'] = record.get('reasons') or draw['reasons']
        if record['stage'] in ('present_call_end', 'present'):
            draw['end'] = max(draw['end'] or 0, record['ns'])
    return [d for d in out.values() if d['end'] is not None]


def summary(values):
    return {'count': len(values), 'p50_ms': round(statistics.median(values), 2) if values else None,
            'p95_ms': round(percentile(values, .95), 2) if values else None,
            'max_ms': round(max(values), 2) if values else None}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--binary', required=True, type=Path)
    ap.add_argument('--output', required=True, type=Path)
    ap.add_argument('--renderer', default='pixman', choices=('pixman', 'gles2'))
    ap.add_argument('--mesa', type=Path, help='Mesa for a gles2 compositor')
    ap.add_argument('--files', type=int, default=200)
    ap.add_argument('--input', default='file-01')
    ap.add_argument('--shrink', default='99')
    ap.add_argument('--slowdown', type=float, default=1)
    ap.add_argument('--shots', action='store_true')
    ap.add_argument('--grim', default='/usr/bin/grim')
    ap.add_argument('--idle', type=float, default=3)
    ap.add_argument('--budget-ms', type=float)
    ap.add_argument('--no-force', action='store_true', help='do not set ROMACS_LAYER_ANIMATION=1')
    ap.add_argument('--expect-none', action='store_true')
    ap.add_argument('--init', type=Path)
    ap.add_argument('--font-size', default='15')
    ap.add_argument('--mode', default='2560x1440')
    ap.add_argument('--timeout', type=float, default=60)
    ap.add_argument('--labwc', default=live.LABWC)
    ap.add_argument('--wlr-randr', default=live.RANDR)
    args = ap.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    trace = out / 'trace.jsonl'
    if trace.exists():
        trace.unlink()
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'renderer': args.renderer, 'forced': not args.no_force, 'phases': {}}
    comp = kb = proc = None
    try:
        with tempfile.TemporaryDirectory(prefix='lan-', dir=os.environ.get('XDG_RUNTIME_DIR')) as tmp:
            root = Path(tmp)
            root.chmod(0o700)
            home, work, config = root / 'home', root / 'work', root / 'config'
            for d in (home, work, config):
                d.mkdir()
            for n in range(args.files):
                (work / f'file-{n:04}.txt').write_text('x\n')
            (config / 'rc.xml').write_text(live.RC_XML)
            (home / 'config' / 'romacs').mkdir(parents=True)
            (home / 'config' / 'romacs' / 'init.el').write_text(args.init.read_text() if args.init else INIT)
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': args.renderer, 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
                   'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': args.font_size,
                   'ROMACS_PRESENT_TRACE': str(trace)}
            if not args.no_force:
                env['ROMACS_LAYER_ANIMATION'] = str(args.slowdown)
            for k in ('CARGO_HOME', 'ROMACS_RUNTIME_ROOT', 'ROMACS_LISP_DIR', 'ROMACS_WORLD'):
                if k in os.environ:
                    env[k] = os.environ[k]
            log_path = out / 'binary.stderr'
            with (out / 'compositor.log').open('w') as clog, log_path.open('w') as log:
                comp_env = dict(env)
                if args.mesa:
                    vendor = root / 'egl-mesa.json'
                    vendor.write_text(json.dumps({'file_format_version': '1.0.0',
                                                  'ICD': {'library_path': str(args.mesa / 'lib/libEGL_mesa.so.0')}}))
                    comp_env.update(__EGL_VENDOR_LIBRARY_FILENAMES=str(vendor),
                                    GBM_BACKENDS_PATH=str(args.mesa / 'lib/gbm'),
                                    LIBGL_DRIVERS_PATH=str(args.mesa / 'lib/dri'))
                comp = subprocess.Popen([args.labwc, '-C', str(config)], env=comp_env, stdout=clog, stderr=clog)
                wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'compositor', comp)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                subprocess.run([args.wlr_randr, '--output', 'HEADLESS-1', '--custom-mode', args.mode],
                               env=env, check=True, timeout=15)
                kb = subprocess.Popen(['/usr/bin/wtype', '-s', '600000', '-k', 'Shift_L'], env=env,
                                      stdout=log, stderr=log)
                time.sleep(.3)
                proc = subprocess.Popen([str(binary), '--loadup'], env=env, cwd=work,
                                        stdin=subprocess.DEVNULL, stdout=log, stderr=log)

                def keys(*parts, delay=40):
                    subprocess.run(['/usr/bin/wtype', '-d', str(delay), *parts], env=env, check=True, timeout=30)

                def ctrl(k):
                    keys('-M', 'ctrl', k, '-m', 'ctrl')

                def painted():
                    return '"diagnostic":"paint' in log_path.read_text(errors='replace')

                wait(painted, max(args.timeout, 600), 'first frame', proc)
                time.sleep(3)
                report['adapter'] = next((l for l in log_path.read_text(errors='replace').split('\n')
                                          if l.startswith('romacs: gpu ')), None)
                marks = []

                def shot(name):
                    if args.shots:
                        subprocess.run([args.grim, str(out / f'{name}.png')], env=env, timeout=30)

                def phase(name, act, pause=1.0):
                    pause *= args.slowdown
                    start = time.clock_gettime_ns(time.CLOCK_MONOTONIC)
                    act()
                    if args.shots and name in ('open', 'move', 'close', 'shrink', 'grow'):
                        time.sleep(.14 * args.slowdown / 3)
                        shot(f'{name}-{len(marks)}-mid')
                    time.sleep(pause)
                    shot(f'{name}-{len(marks)}-end')
                    marks.append((name, start, time.clock_gettime_ns(time.CLOCK_MONOTONIC)))

                phase('open', lambda: (ctrl('x'), ctrl('f')))
                for ch in args.input:
                    phase('type', lambda ch=ch: keys(*(('-k', KEYSYMS[ch]) if ch in KEYSYMS else (ch,)), delay=10), .5)
                for ch in args.shrink:
                    phase('shrink', lambda ch=ch: keys(ch, delay=10), .5)
                for _ in args.shrink:
                    phase('grow', lambda: keys('-k', 'BackSpace', delay=10), .5)
                for _ in range(3):
                    phase('move', lambda: ctrl('n'), .5)
                phase('close', lambda: ctrl('g'))
                phase('idle', lambda: None, args.idle)
                time.sleep(.5)
                all_draws = draws(trace)
                for name, start, end in marks:
                    inside = [d for d in all_draws if start <= d['start'] < end]
                    motion = [(d['end'] - d['start']) / 1e6 for d in inside if 'ChildFrameMotion' in d['reasons']]
                    entry = report['phases'].setdefault(name, {'draws': 0, 'motion': []})
                    entry['draws'] += len(inside)
                    entry['motion'] += motion
                for entry in report['phases'].values():
                    entry['motion'] = summary(entry.pop('motion'))
                motion_all = [(d['end'] - d['start']) / 1e6 for d in all_draws if 'ChildFrameMotion' in d['reasons']]
                report['motion'] = summary(motion_all)
                idle_draws = report['phases']['idle']['draws']
                ok = True
                if args.expect_none:
                    ok = report['motion']['count'] == 0
                if args.budget_ms is not None:
                    p95 = report['motion']['p95_ms']
                    ok = ok and p95 is not None and p95 <= args.budget_ms and idle_draws == 0
                report['status'] = 'PASS' if ok else 'FAIL'
                ctrl('x')
                ctrl('c')
                time.sleep(1)
    except Exception as error:
        report.update(status='FAIL', error=repr(error))
    finally:
        for p in (proc, kb, comp):
            if p is not None and p.poll() is None:
                p.terminate()
                try:
                    p.wait(5)
                except subprocess.TimeoutExpired:
                    p.kill()
        (out / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
        print(json.dumps(report))
    return 0 if report.get('status') == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
