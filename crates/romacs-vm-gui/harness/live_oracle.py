#!/usr/bin/env python3
"""G1a compositor-only acceptance; JSON replies synchronize, never grade pixels."""
import argparse
import contextlib
import dataclasses
import json
import math
import os
from pathlib import Path
import re
import select
import shutil
import subprocess
import tempfile
import time

WIDTH, HEIGHT = 400, 300
SOURCE = '(with-current-buffer "*g1a*" (goto-char (point-max)) (insert "Δ"))'
LABWC = os.environ.get("ROMACS_TEST_LABWC", "labwc")
RANDR = os.environ.get("ROMACS_TEST_WLR_RANDR", "wlr-randr")
# Init text for a harness that matches screen rows by their glyphs: an
# interactive session's defaults number programming buffers' lines
# (VM2-OOB-DEFAULTS), whose digits are ordinary text glyphs, so a harness
# reading a programming buffer's rows turns the numbers off the way an init
# does.
NO_LINE_NUMBERS = """;; The rows this harness reads are the text's alone, no line-number column.
(add-hook 'prog-mode-hook (lambda () (display-line-numbers-mode -1)) 90)
"""
RC_XML = '''<?xml version="1.0"?>
<labwc_config>
  <core><gap>0</gap></core>
  <windowRules><windowRule identifier="*" serverDecoration="no">
    <action name="Maximize"/>
  </windowRule></windowRules>
</labwc_config>
'''


def resolve_font(family: str = 'Liberation Mono') -> Path:
    """Resolve FAMILY to a file, refusing a host-specific substitution.

    `fc-match` consults the *user's* fontconfig, so this resolution is not
    hermetic: ~/.config/fontconfig/fonts.conf can remap the system Liberation
    Mono query to a personal typeface. The substitution is silent, and its
    only symptom is a pixel read that disagrees by a pixel and template-matches
    neighbouring glyphs into garbage -- a deterministic false RED ten minutes
    into a run, blamed on the editor rather than on the font.

    An acceptance oracle whose verdict depends on ambient user configuration is
    not evidence, so fail here, at the cause, naming both paths and the fix.
    """
    found = subprocess.check_output(
        ['fc-match', '-f', '%{file}', family]).decode().strip()
    if not found:
        raise SystemExit(f'fc-match returned no file for {family!r}')
    path = Path(found)
    expected = family.replace(' ', '') + '-Regular'
    if expected.lower() not in path.name.lower():
        raise SystemExit(
            f'fontconfig substituted {path} for {family!r}.\n'
            f'The scene expectations are authored against {expected}, so the '
            f'pixel templates would be built for the wrong typeface and every '
            f'glyph read would fail misleadingly.\n'
            f'Pass --font explicitly, or provide isolated fontconfig settings '
            f'for this test process that resolve the expected system font.')
    return path


def font_metrics(font: Path, magick: str) -> dict:
    """Independent FreeType sizing through ImageMagick, at exactly 20 pixels/em."""
    result = subprocess.run(
        [magick, '-debug', 'annotate', '-density', '72', '-size', '32x32',
         'xc:black', '-font', str(font), '-pointsize', '20', '-annotate', '+0+0',
         '0', 'null:'], capture_output=True, text=True, check=True, timeout=10)
    match = re.search(r'Metrics: text: 0; width: ([\d.]+);.*?ascent: ([\d.]+); '
                      r'descent: (-?[\d.]+);', result.stderr)
    if not match:
        raise RuntimeError(f'cannot independently size font: {result.stderr}')
    width, ascent, descent = map(float, match.groups())
    row_height = math.ceil(ascent + abs(descent))
    baseline = math.floor(ascent + 0.5)
    cell_width = math.floor(width + 0.5)
    if min(baseline, cell_width, row_height - baseline) <= 0:
        raise ValueError('font returned invalid cell metrics')
    return {'ascent': baseline, 'descent': row_height - baseline,
            'cell_width': cell_width}


def scene(font: Path, metrics: dict, *, after: bool) -> dict:
    rows = [{'text': 'Ωμέγα Hello',
             'runs': [{'start': 6, 'end': 11, 'fg': '#ff0000'}]}]
    if after:
        rows.append({'text': 'Δ'})
    return {'name': 'g1a-after' if after else 'g1a-before',
            'frame': {'width': WIDTH, 'height': HEIGHT, 'bg': '#000000',
                      'fg': '#ffffff', 'fringe': 0},
            'font': {'family': 'Liberation Mono', 'pixel_size': 20,
                     'regular': str(font), **metrics},
            'cursor_color': '#00a000',
            'buffer': [{'text': 'Ωμέγα '}, {'text': 'Hello', 'fg': '#ff0000'},
                       {'text': '\nΔ' if after else '\n'}],
            'expect': {'origin': [0, 0], 'rows': rows, 'cursor': None}}


def save_json(path: Path, value) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')


class ReplyReader:
    def __init__(self, proc: subprocess.Popen, timeout: float):
        self.proc = proc
        self.timeout = timeout
        self.pending = b''

    def read(self) -> dict:
        deadline = time.monotonic() + self.timeout
        if self.proc.stdout is None:
            raise RuntimeError('binary stdout is not piped')
        while b'\n' not in self.pending:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.proc.stdout], [], [], remaining)[0]:
                raise RuntimeError('timeout waiting for presentation activation JSON')
            chunk = os.read(self.proc.stdout.fileno(), 65536)
            if not chunk:
                raise RuntimeError(f'binary stdout closed (exit={self.proc.poll()})')
            self.pending += chunk
            if len(self.pending) > 1024 * 1024:
                raise ValueError('binary reply exceeds 1 MiB')
        line, self.pending = self.pending.split(b'\n', 1)
        reply = json.loads(line)
        if not isinstance(reply, dict):
            raise ValueError('binary reply is not a JSON object')
        return reply


def presentation(reply: dict) -> int:
    value = reply.get('presentation')
    if type(value) is not int or value < 1:
        raise ValueError(f'missing positive presentation activation: {reply}')
    return value


def rgb(path: Path, magick: str) -> bytes:
    size = subprocess.check_output([magick, 'identify', '-format', '%w %h', str(path)],
                                   timeout=10).decode()
    if size != f'{WIDTH} {HEIGHT}':
        raise ValueError(f'expected {WIDTH}x{HEIGHT} compositor capture, got {size}')
    return subprocess.check_output([magick, str(path), '-alpha', 'off', '-depth', '8',
                                    'RGB:-'], timeout=10)


def write_rgb(path: Path, pixels: bytes | bytearray, magick: str) -> None:
    subprocess.run([magick, '-size', f'{WIDTH}x{HEIGHT}', '-depth', '8', 'RGB:-',
                    str(path)], input=pixels, check=True, timeout=10)


def make_mutants(before: Path, after: Path, out: Path, magick: str, metrics: dict) -> dict:
    pixels = rgb(after, magick)
    wrong = bytearray(pixels)
    changed = 0
    row_height = metrics['ascent'] + metrics['descent']
    for y in range(row_height):
        for x in range(6 * metrics['cell_width'], 11 * metrics['cell_width']):
            i = (y * WIDTH + x) * 3
            red, green, blue = wrong[i:i + 3]
            if red > max(green, blue) + 8:
                wrong[i:i + 3] = bytes((green, red, blue))
                changed += 1
    if not changed:
        raise RuntimeError('wrong-colour mutant found no red Hello ink')
    shifted = bytearray(len(pixels))
    stride = WIDTH * 3
    for y in range(HEIGHT):
        start = y * stride
        shifted[start + 3:start + stride] = pixels[start:start + stride - 3]
    paths = {name: out / f'mutant-{name}.png'
             for name in ('wrong-colour', 'shift-1px', 'delta-absent')}
    write_rgb(paths['wrong-colour'], wrong, magick)
    write_rgb(paths['shift-1px'], shifted, magick)
    # The real pre-mutation compositor capture is the strongest absent-Delta control.
    shutil.copyfile(before, paths['delta-absent'])
    return paths


def judge(oracle: str, spec: Path, capture: Path, report: Path, *, reject: bool) -> dict:
    result = subprocess.run([oracle, 'check', str(spec), str(capture)],
                            capture_output=True, text=True, timeout=120)
    record = {'command': [oracle, 'check', str(spec), str(capture)],
              'exit': result.returncode, 'stdout': result.stdout, 'stderr': result.stderr}
    save_json(report, record)
    expected = 1 if reject else 0
    if result.returncode != expected:
        raise RuntimeError(f'{capture.name}: oracle exit {result.returncode}, '
                           f'expected {expected}; see {report}')
    return record


# How long a process ending is given before it counts as hung: a loaded
# host has taken over ten seconds to end a session, a hang takes forever.
HANG = 120


# The environment every live session runs in, besides its own paths.
SESSION_ENV = {'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1', 'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman',
               'WLR_HEADLESS_OUTPUTS': '1', 'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop'}
# What a gate run sets that a session must keep (its Lisp, world and fonts).
SESSION_PASSED = ('ROMACS_LISP_DIR', 'ROMACS_RUNTIME_ROOT', 'ROMACS_WORLD', 'ROMACS_GATE_FONTCONFIG')


@dataclasses.dataclass
class Session:
    """A running live session: its private ROOT (HOME, WORK, CONFIG), the
    ENV its clients use, the binary's PROCESS and its LOG."""
    root: Path
    home: Path
    work: Path
    config: Path
    env: dict
    log: Path
    process: subprocess.Popen | None = None
    processes: list = dataclasses.field(default_factory=list)


@contextlib.contextmanager
def session(binary, output, *, prefix, init=None, args=('--loadup', '--trace-scene'), rc_xml=RC_XML,
            env=None, size='1280x800', tools=None, prepare=None, report=None, log='binary.stderr', compositor_env=None):
    """One live session, its whole life in order: a private root (HOME,
    WORK, CONFIG; INIT as the init file), a headless labwc on RC_XML sized
    SIZE, a held key for the seat's keyboard, BINARY with ARGS run from
    WORK, logging to OUTPUT/LOG.  PREPARE(session) writes what the session
    needs before the binary starts.  Yields the Session.  After the body,
    or whatever it raised: the processes stopped and reaped, REPORT (a dict)
    written to OUTPUT/report.json with any error the body raised, then the
    root removed: never while something still writes into it.  An error
    the body raised is recorded in REPORT, not raised, when REPORT is given;
    one before the session started is recorded and raised. COMPOSITOR_ENV
    is labwc's alone (its renderer on a real GPU)."""
    root, processes = None, []
    started = False
    try:
        root = Path(tempfile.mkdtemp(prefix=prefix, dir=os.environ.get('XDG_RUNTIME_DIR')))
        root.chmod(0o700)
        home, work, config = root / 'home', root / 'work', root / 'config'
        for directory in (home / 'config/romacs', work, config):
            directory.mkdir(parents=True)
        if init is not None:
            (home / 'config/romacs/init.el').write_text(init)
        (config / 'rc.xml').write_text(rc_xml)
        # The gate's headless tools (labwc, wlr-randr) sit together.
        path = ':'.join(filter(None, [tools, os.path.dirname(LABWC) if os.path.isabs(LABWC) else None,
                                      os.environ.get('PATH', '/usr/bin:/bin')]))
        live_env = {**SESSION_ENV, 'PATH': path, 'HOME': str(home),
                    'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                    'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                    'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root)}
        live_env.update({key: os.environ[key] for key in SESSION_PASSED if key in os.environ})
        if 'ROMACS_GATE_FONTCONFIG' in live_env:
            live_env['FONTCONFIG_FILE'] = live_env['ROMACS_GATE_FONTCONFIG']
        live_env.update(env or {})
        log_path = Path(output) / log
        current = Session(root, home, work, config, live_env, log_path, processes=processes)
        if prepare is not None:
            prepare(current)
        with (Path(output) / 'compositor.log').open('w') as compositor_log, log_path.open('w') as binary_log:
            processes.append(subprocess.Popen([shutil.which('labwc', path=live_env['PATH']) or LABWC, '-C', str(config)],
                                              env=live_env | (compositor_env or {}), stdout=compositor_log, stderr=compositor_log))
            wait_socket = time.monotonic() + 60
            while not any(p.is_socket() for p in root.glob('wayland-*')):
                if processes[0].poll() is not None or time.monotonic() > wait_socket:
                    raise RuntimeError('the private compositor did not start')
                time.sleep(.02)
            live_env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
            subprocess.run([shutil.which('wlr-randr', path=live_env['PATH']) or RANDR, '--output', 'HEADLESS-1',
                            '--custom-mode', size], env=live_env, check=True, timeout=60)
            # A held key gives the seat a keyboard before the client maps.
            processes.append(subprocess.Popen(['wtype', '-s', '86400000', '-k', 'Shift_L'], env=live_env,
                                              stdout=binary_log, stderr=binary_log))
            time.sleep(.3)
            current.process = subprocess.Popen([str(Path(binary).resolve()), *args], env=live_env, cwd=work,
                                               stdin=subprocess.DEVNULL, stdout=binary_log, stderr=binary_log)
            processes.append(current.process)
            started = True
            yield current
    except Exception as raised:  # noqa: BLE001 - recorded in the report when there is one
        if report is not None:
            report['error'] = repr(raised)
        # The body's error is the report's; one before the session started
        # has no body to return to.
        if report is None or not started:
            raise
    finally:
        # Every process is stopped whatever another's stop raised; the root
        # goes only once all are reaped, never under one still running.
        unstopped = []
        for process in reversed(processes):
            try:
                stop(process)
            except Exception as failed:  # noqa: BLE001 - raised below, after the report
                unstopped.append(failed)
        if report is not None and unstopped:
            report.setdefault('error', repr(unstopped[0]))
        try:
            if report is not None:
                (Path(output) / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
        finally:
            if not unstopped:
                remove_root(root)
        if unstopped:
            raise unstopped[0]


def remove_root(path) -> None:
    """Remove PATH, a session's private directory (its runtime dir, HOME,
    sockets), once its processes are stopped: a `TemporaryDirectory' that
    removed itself as its `with' ended raced the compositor and the session
    still writing into it (ENOTEMPTY). Opened with `delete=False', it is
    removed here, in the `finally' after the stops."""
    if path is not None:
        shutil.rmtree(path)


def stop(proc) -> None:
    """End PROC, if it runs: SIGTERM, then SIGKILL once it has had HANG
    seconds to go."""
    if proc is not None and proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=HANG)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=HANG)


def capture(path: Path, env: dict, args, proc) -> None:
    # Activation is the ordering boundary; settling only drains compositor work.
    time.sleep(args.settle)
    if proc.poll() is not None:
        raise RuntimeError(f'binary exited {proc.returncode} before capture')
    subprocess.run([args.grim, str(path)], env=env, check=True, timeout=10)
    rgb(path, args.magick)


def run_variant(args, name: str, flags: list, metrics: dict, specs: dict) -> dict:
    out = args.output / name
    out.mkdir()
    with tempfile.TemporaryDirectory(prefix='g1a-', dir=args.runtime_root) as temp:
        runtime = Path(temp)
        runtime.chmod(0o700)
        config = runtime / 'config'
        config.mkdir()
        (config / 'rc.xml').write_text(RC_XML)
        home = runtime / 'home'
        home.mkdir()
        for directory in ('config', 'cache', 'data', 'state'):
            (home / directory).mkdir()
        env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home),
               'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1', 'XDG_RUNTIME_DIR': str(runtime),
               'XDG_CONFIG_HOME': str(home / 'config'),
               'XDG_CACHE_HOME': str(home / 'cache'),
               'XDG_DATA_HOME': str(home / 'data'),
               'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(runtime),
               'WLR_BACKENDS': 'headless', 'WLR_HEADLESS_OUTPUTS': '1',
               'WLR_RENDERER': 'pixman', 'WLR_LIBINPUT_NO_DEVICES': '1',
               'LIBSEAT_BACKEND': 'noop'}
        proc = comp = None
        with (out / 'compositor.log').open('w') as comp_log, \
                (out / 'binary.stderr').open('w') as binary_log:
            try:
                comp = subprocess.Popen([args.labwc, '-C', str(config)], env=env,
                                        stdout=comp_log, stderr=comp_log)
                deadline = time.monotonic() + args.timeout
                while True:
                    sockets = sorted(p for p in runtime.glob('wayland-*') if p.is_socket())
                    if sockets:
                        break
                    if comp.poll() is not None or time.monotonic() >= deadline:
                        raise RuntimeError(f'{name}: private compositor failed; see {out}')
                    time.sleep(0.05)
                socket = sockets[0]
                env['WAYLAND_DISPLAY'] = socket.name
                subprocess.run([args.wlr_randr, '--output', 'HEADLESS-1', '--custom-mode',
                                f'{WIDTH}x{HEIGHT}'], env=env, check=True, timeout=10,
                               stdout=subprocess.DEVNULL)
                command = [args.binary, '--wayland', str(socket), *flags]
                save_json(out / 'launch.json', {'command': command, 'output': [WIDTH, HEIGHT]})
                proc = subprocess.Popen(command, env=env, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=binary_log)
                reader = ReplyReader(proc, args.timeout)
                ready = reader.read()
                save_json(out / 'ready.json', ready)
                if ready.get('ready') is not True or 'error' in ready:
                    raise ValueError(f'{name}: invalid ready reply {ready}')
                first = presentation(ready)
                before = out / 'before.png'
                capture(before, env, args, proc)
                judge(args.oracle, specs['before'], before, out / 'before-check.json', reject=False)
                request = {'source': SOURCE}
                save_json(out / 'request.json', request)
                if proc.stdin is None:
                    raise RuntimeError('binary stdin is not piped')
                proc.stdin.write((json.dumps(request, ensure_ascii=False) + '\n').encode())
                proc.stdin.flush()
                reply = reader.read()
                save_json(out / 'reply.json', reply)
                if 'result' not in reply or 'error' in reply or presentation(reply) <= first:
                    raise ValueError(f'{name}: mutation did not activate a new presentation: {reply}')
                after = out / 'after.png'
                capture(after, env, args, proc)
                judge(args.oracle, specs['after'], after, out / 'after-check.json', reject=False)
                mutants = make_mutants(before, after, out, args.magick, metrics)
                for kind, path in mutants.items():
                    judge(args.oracle, specs['after'], path, out / f'{kind}-check.json', reject=True)
                return {'variant': name, 'pass': True, 'presentations': [first, reply['presentation']],
                        'negative_controls': list(mutants)}
            finally:
                stop(proc)
                if proc is not None:
                    if proc.stdin is not None:
                        proc.stdin.close()
                    if proc.stdout is not None:
                        proc.stdout.close()
                stop(comp)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--oracle', required=True)
    parser.add_argument('--output', type=Path, required=True,
                        help='new artifact directory (must not already exist)')
    parser.add_argument('--labwc', default=LABWC)
    parser.add_argument('--wlr-randr', default=RANDR)
    parser.add_argument('--grim', default='grim')
    parser.add_argument('--magick', default='magick')
    parser.add_argument('--font', type=Path,
                        default=Path('/usr/share/fonts/liberation/LiberationMono-Regular.ttf'))
    parser.add_argument('--runtime-root', type=Path,
                        default=Path(tempfile.gettempdir()))
    parser.add_argument('--gc-floor', type=int, default=1)
    parser.add_argument('--timeout', type=float, default=45)
    parser.add_argument('--settle', type=float, default=0.5)
    args = parser.parse_args()
    if args.gc_floor < 1 or args.timeout <= 0 or args.settle < 0:
        parser.error('gc-floor and timeout must be positive; settle must be nonnegative')
    for field in ('binary', 'oracle', 'labwc', 'wlr_randr', 'grim', 'magick'):
        executable = shutil.which(getattr(args, field))
        if executable is None:
            parser.error(f'{field}: executable not found: {getattr(args, field)}')
        setattr(args, field, str(Path(executable).resolve()))
    args.font = args.font.resolve(strict=True)
    args.output = args.output.resolve()
    args.runtime_root = args.runtime_root.resolve(strict=True)
    # wl_display_add_socket's sockaddr_un includes the runtime prefix.
    if len(os.fsencode(str(args.runtime_root))) + 35 >= 108:
        parser.error('runtime-root is too long for a private Unix socket')
    args.output.mkdir(parents=True, exist_ok=False)
    summary = {'pass': False, 'runs': [], 'claim': 'Text exact up to Liberation Mono identical pairs; '
               'H/Η and o/ο occur in this scene. No renderer diagnostics grade pixels.'}
    try:
        metrics = font_metrics(args.font, args.magick)
        save_json(args.output / 'independent-metrics.json',
                  {'font': str(args.font), 'pixel_size': 20, 'origin': [0, 0],
                   'measurement': 'ImageMagick FreeType at density 72', **metrics})
        specs = {}
        for phase in ('before', 'after'):
            specs[phase] = args.output / f'{phase}.json'
            save_json(specs[phase], scene(args.font, metrics, after=phase == 'after'))
        variants = [('normal', []), ('low-gc', ['--gc-floor', str(args.gc_floor)]),
                    ('forced-gc', ['--gc-floor', str(args.gc_floor),
                                   '--gc-between-snapshot-and-paint'])]
        for name, flags in variants:
            summary['runs'].append(run_variant(args, name, flags, metrics, specs))
            if name != 'normal':
                for phase in ('before', 'after'):
                    if rgb(args.output / name / f'{phase}.png', args.magick) != rgb(
                            args.output / 'normal' / f'{phase}.png', args.magick):
                        raise RuntimeError(f'{name}: {phase} capture differs from normal run')
        summary['pass'] = True
        print(json.dumps(summary, ensure_ascii=False))
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        summary['error'] = str(error)
        print(json.dumps(summary, ensure_ascii=False))
        return 1
    finally:
        save_json(args.output / 'summary.json', summary)


if __name__ == '__main__':
    raise SystemExit(main())
