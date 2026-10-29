#!/usr/bin/env python3
"""A --loadup session's world by default, in a private headless compositor.

With no world flag, a live --loadup launch keeps its world in
$XDG_CACHE_HOME/romacs/worlds. The first launch in a fresh HOME finds none,
says so and paints "Preparing Romacs (first start)…" before it waits,
builds the world and boots from it. A later launch restores it without
building, and after its first frame prunes a week-old world planted beside
it. One with ROMACS_WORLD=off boots from source and leaves the store alone.
Each launch must reach its first frame. The binary's own lines say which:
`romacs-world: {"mode": ...}`, `{"event": "preparing"...}`, `{"event":
"pruned"...}`.

Every process gets a private HOME/XDG tree and the private compositor's
WAYLAND_DISPLAY; DISPLAY is never set. labwc and wlr-randr come from
--tools (the gate's headless tools) or PATH.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import fileio_live as fio
import find_file_live as ff
import live_oracle as live


def events(text, event):
    """The binary's world events named EVENT."""
    return [json.loads(line.split(': ', 1)[1]) for line in text.splitlines()
            if line.startswith('romacs-world: {') and f'"event":"{event}"' in line]


def world_line(text):
    """The binary's world line: how this launch booted."""
    for line in text.splitlines():
        if line.startswith('romacs-world: {') and '"mode"' in line:
            return json.loads(line.split(': ', 1)[1])
    return None


def store_state(store):
    """Each file of the store, but the identity cache, with its mtime."""
    if not store.exists():
        return {}
    return {p.name: p.stat().st_mtime_ns for p in store.iterdir() if p.is_file()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', help='directory with labwc and wlr-randr')
    parser.add_argument('--timeout', type=float, default=180)
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'launches': [], 'checks': [], 'failures': []}
    compositor = None
    try:
        with tempfile.TemporaryDirectory(prefix='world-live-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            home, work, config = [root / name for name in ('home', 'work', 'config')]
            for directory in (home, work, config):
                directory.mkdir()
            (config / 'rc.xml').write_text(live.RC_XML)
            path = os.environ.get('PATH', '/usr/bin:/bin')
            env = {'PATH': f'{args.tools}:{path}' if args.tools else path,
                   'HOME': str(home), 'LANG': 'C.UTF-8',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop'}
            for key in ('ROMACS_LISP_DIR', 'ROMACS_RUNTIME_ROOT', 'ROMACS_GATE_FONTCONFIG'):
                if key in os.environ:
                    env[key] = os.environ[key]
            if 'ROMACS_GATE_FONTCONFIG' in env:
                env['FONTCONFIG_FILE'] = env['ROMACS_GATE_FONTCONFIG']
            store = home / 'cache/romacs/worlds'

            def check(name, ok, detail):
                report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
                if not ok:
                    report['failures'].append(name)

            with (output / 'compositor.log').open('w') as compositor_log:
                compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or live.LABWC, '-C', str(config)],
                                              env=env, stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                assert 'DISPLAY' not in env and (root / env['WAYLAND_DISPLAY']).is_socket()
                subprocess.run(['wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1280x800'],
                               env=env, check=True, timeout=15)

                def launch(name, extra_env):
                    """Run --loadup to its first frame: how it booted and how long that took."""
                    log_path = output / f'{name}.stderr'
                    with log_path.open('w') as log:
                        started = time.monotonic()
                        process = subprocess.Popen([str(binary), '--loadup', '--trace-scene', '-Q'],
                                                   env={**env, **extra_env}, cwd=work,
                                                   stdin=subprocess.DEVNULL, stdout=log, stderr=log)
                        try:
                            if name == 'first':
                                # The window while the world builds, for review.
                                fio.wait(lambda: '"event":"preparing-frame"' in log_path.read_text(errors='replace'),
                                         args.timeout, f'{name}: preparing', process)
                                time.sleep(1.5)
                                fio.shot(output / 'preparing.png', env, report)
                            fio.wait(lambda: ff.current_rows(log_path.read_text(errors='replace')),
                                     args.timeout, f'{name}: first frame', process)
                            seconds = time.monotonic() - started
                            # Pruning follows the first frame, off its path.
                            time.sleep(1.5)
                        finally:
                            process.terminate()
                            try:
                                process.wait(timeout=30)
                            except subprocess.TimeoutExpired:
                                process.kill()
                                process.wait()
                    text = log_path.read_text(errors='replace')
                    entry = {'launch': name, 'seconds': round(seconds, 3), 'world': world_line(text),
                             'preparing': events(text, 'preparing-frame'), 'pruned': events(text, 'pruned')}
                    report['launches'].append(entry)
                    return entry

                first = launch('first', {})
                built = store_state(store)
                check('a first launch builds its world in XDG_CACHE_HOME/romacs/worlds and boots from it',
                      (first['world'] or {}).get('mode') == 'restored'
                      and any(n.endswith('.rwi') for n in built) and any(n.endswith('.json') for n in built), first)
                check('the default store is private', store.exists() and store.stat().st_mode & 0o777 == 0o700,
                      oct(store.stat().st_mode & 0o777) if store.exists() else None)
                check('a first launch says it prepares, and paints that, before it waits',
                      [p['painted'] for p in first['preparing']] == [True], first['preparing'])
                # A world of another build, unused for a week.
                old = 'e' * 64
                week_ago = time.time() - 8 * 86400
                for suffix in ('rwi', 'json', 'log', 'lock'):
                    path = store / f'{old}.{suffix}'
                    path.write_text('old')
                    os.utime(path, (week_ago, week_ago))
                built = store_state(store)
                later = launch('later', {})
                after = store_state(store)
                check('a later launch restores the world without building, and says nothing of preparing',
                      (later['world'] or {}).get('mode') == 'restored' and not later['preparing']
                      and {k: v for k, v in after.items() if k in built and not k.startswith(old)}
                      == {k: v for k, v in built.items() if not k.startswith(old) and not k.endswith('.use')} | {k: after[k] for k in after if k.endswith('.use')},
                      {'launch': later})
                check('after its first frame, a launch prunes a world unused for a week',
                      not any(k.startswith(old) for k in after) and later['pruned'] == [{'event': 'pruned', 'worlds': [old]}],
                      {'store': sorted(after), 'pruned': later['pruned']})
                built = after
                check('a later launch is faster than the first', later['seconds'] < first['seconds'],
                      [first['seconds'], later['seconds']])
                off = launch('off', {'ROMACS_WORLD': 'off'})
                check('ROMACS_WORLD=off boots from source and leaves the store alone',
                      off['world'] == {'mode': 'source', 'reason': 'ROMACS_WORLD=off'} and store_state(store) == built
                      and not off['preparing'], off)
        report['status'] = 'PASS' if not report['failures'] else 'FAIL'
    except Exception as error:  # noqa: BLE001 - the report carries it
        report['error'] = repr(error)
    finally:
        if compositor is not None:
            compositor.terminate()
            try:
                compositor.wait(timeout=10)
            except subprocess.TimeoutExpired:
                compositor.kill()
        (output / 'report.json').write_text(json.dumps(report, indent=2))
    print(report['status'], json.dumps({'launches': report['launches'], 'failures': report['failures'],
                                        'error': report.get('error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
