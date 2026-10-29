#!/usr/bin/env python3
"""A --loadup session's world by default, in a private headless compositor.

With no world flag, a live --loadup launch keeps its world in
$XDG_CACHE_HOME/romacs/worlds. The first launch in a fresh HOME finds none,
builds it and boots from it; a later launch restores it without building;
one with ROMACS_WORLD=off boots from source and leaves the store alone. Each
launch must reach its first frame. The binary's own line says which:
`romacs-world: {"mode": ...}`.

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
                            fio.wait(lambda: ff.current_rows(log_path.read_text(errors='replace')),
                                     args.timeout, f'{name}: first frame', process)
                            seconds = time.monotonic() - started
                        finally:
                            process.terminate()
                            try:
                                process.wait(timeout=30)
                            except subprocess.TimeoutExpired:
                                process.kill()
                                process.wait()
                    world = world_line(log_path.read_text(errors='replace'))
                    entry = {'launch': name, 'seconds': round(seconds, 3), 'world': world}
                    report['launches'].append(entry)
                    return entry

                first = launch('first', {})
                built = store_state(store)
                check('a first launch builds its world in XDG_CACHE_HOME/romacs/worlds and boots from it',
                      (first['world'] or {}).get('mode') == 'restored'
                      and any(n.endswith('.rwi') for n in built) and any(n.endswith('.json') for n in built), first)
                check('the default store is private', store.exists() and store.stat().st_mode & 0o777 == 0o700,
                      oct(store.stat().st_mode & 0o777) if store.exists() else None)
                later = launch('later', {})
                check('a later launch restores the world without building', (later['world'] or {}).get('mode') == 'restored'
                      and store_state(store) == built, {'launch': later, 'store_unchanged': store_state(store) == built})
                check('a later launch is faster than the first', later['seconds'] < first['seconds'],
                      [first['seconds'], later['seconds']])
                off = launch('off', {'ROMACS_WORLD': 'off'})
                check('ROMACS_WORLD=off boots from source and leaves the store alone',
                      off['world'] == {'mode': 'source', 'reason': 'ROMACS_WORLD=off'} and store_state(store) == built, off)
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
