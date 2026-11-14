#!/usr/bin/env python3
"""Window ends across JSON requests in the --wayland transport.

One private headless labwc and `romacs-vm-gui --wayland SOCKET`.  The first
request fills the shown buffer past the window and is painted, which applies
the layout's window ends.  The second selects the buffer the window already
shows: that resets the window's end, though the frame stays the same.  The
third asks `window-end`, which must still be the painted end, not the
buffer's end.

usage: requests_live.py --binary PATH --output DIR [--labwc PATH] [--wlr-randr PATH]
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

import live_oracle as live

REQUESTS = [
    '(progn (erase-buffer) (let ((i 0)) (while (< i 200) (insert "a line of text\\n") (setq i (1+ i)))) (goto-char 1) nil)',
    '(progn (set-window-buffer nil (current-buffer)) nil)',
    '(list (window-end) (point-max))',
]


def ends(reply):
    """(window-end point-max) from the third request's printed value."""
    end, point_max = reply['result']['value'].strip('()').split()
    return int(end), int(point_max)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--labwc', default=live.LABWC)
    parser.add_argument('--wlr-randr', default=live.RANDR)
    parser.add_argument('--timeout', type=float, default=30)
    args = parser.parse_args()
    args.output.mkdir(parents=True)
    report = {'binary': str(Path(args.binary).resolve()), 'status': 'FAIL', 'replies': []}
    comp = proc = None
    temp = None
    try:
        with tempfile.TemporaryDirectory(prefix='requests-', dir=os.environ.get('XDG_RUNTIME_DIR'), delete=False) as temp:
            runtime = Path(temp)
            runtime.chmod(0o700)
            (runtime / 'config').mkdir()
            (runtime / 'config' / 'rc.xml').write_text(live.RC_XML)
            home = runtime / 'home'
            home.mkdir()
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
                   'XDG_RUNTIME_DIR': str(runtime), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(runtime),
                   'WLR_BACKENDS': 'headless', 'WLR_HEADLESS_OUTPUTS': '1', 'WLR_RENDERER': 'pixman',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop'}
            with (args.output / 'compositor.log').open('w') as comp_log, \
                    (args.output / 'binary.stderr').open('w') as binary_log:
                comp = subprocess.Popen([args.labwc, '-C', str(runtime / 'config')], env=env,
                                        stdout=comp_log, stderr=comp_log)
                deadline = time.monotonic() + args.timeout
                while not any(p.is_socket() for p in runtime.glob('wayland-*')):
                    if comp.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError('private compositor failed')
                    time.sleep(.05)
                socket = next(p for p in runtime.glob('wayland-*') if p.is_socket())
                env['WAYLAND_DISPLAY'] = socket.name
                subprocess.run([args.wlr_randr, '--output', 'HEADLESS-1', '--custom-mode', f'{live.WIDTH}x{live.HEIGHT}'],
                               env=env, check=True, timeout=10, stdout=subprocess.DEVNULL)
                proc = subprocess.Popen([args.binary, '--wayland', str(socket)], env=env, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=binary_log)
                reader = live.ReplyReader(proc, args.timeout)
                if reader.read().get('ready') is not True:
                    raise RuntimeError('binary not ready')
                for source in REQUESTS:
                    proc.stdin.write((json.dumps({'source': source}) + '\n').encode())
                    proc.stdin.flush()
                    reply = reader.read()
                    report['replies'].append(reply)
                    if 'error' in reply:
                        raise RuntimeError(f'request failed: {reply}')
                end, point_max = ends(report['replies'][-1])
                report.update(window_end=end, point_max=point_max, status='PASS' if end < point_max else 'FAIL')
    except Exception as error:  # noqa: BLE001
        report['error'] = repr(error)
    finally:
        live.stop(proc)
        live.stop(comp)
        try:
            (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
        finally:
            live.remove_root(temp)
    print(report['status'], json.dumps({k: report.get(k) for k in ('window_end', 'point_max', 'error')}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
