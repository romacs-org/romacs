#!/usr/bin/env python3
"""Per-key cost of a completion prompt, and terminal pty stability across it.

One private headless labwc (2560x1440, Iosevka 15 by default) and one
`romacs-vm-gui --loadup --trace-latency` in a work directory of --files
files.  The session opens `romacs-term` from M-x, whose shell appends
`stty size` to a file at start and on every SIGWINCH.  Then --cycles times:
C-x C-f, the keys of --input one by one (each waits for its
prompt-key-latency record), C-g.  Last, after an editor command (C-g) in the terminal buffer, the
shell appends `stty size` again.

Reports, per key and as p50/p95 over all keys: command (key read to
redisplay start: Lisp, hooks and the VM snapshot), snapshot, layout,
render wait and key->render, in ms.  The terminal probe reports the pty
size at creation, after the prompts, and every WINCH.  With --assert-stable
the run fails when the pty changed size across the prompts.

--init FILE is the session's init file.  The report's `completion' records
what was in effect inside the first prompt: the completion styles for its
metadata and the icomplete modes; --expect-styles fails the run unless the
styles are exactly the given ones.

usage: completion_perkey_live.py --binary BIN --output DIR [--files 5000]
       [--input file-1] [--cycles 3] [--font-size 15] [--assert-stable]
       [--init FILE] [--expect-styles 'orderless basic']
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
# Records, once, what completion the first prompt used (run in the session).
PROBE = '''
(add-hook 'minibuffer-setup-hook
          (lambda ()
            (add-hook 'post-command-hook
                      (lambda ()
                        (unless (file-exists-p %(path)s)
                          (write-region
                           (prin1-to-string
                            (list :styles (completion--styles
                                           (completion--field-metadata (minibuffer-prompt-end)))
                                  :fido (bound-and-true-p fido-mode)
                                  :icomplete (bound-and-true-p icomplete-mode)
                                  :vertical (bound-and-true-p icomplete-vertical-mode)
                                  :romacs-complete (bound-and-true-p romacs-complete-mode)))
                           nil %(path)s nil 'quiet)))
                      90 t)))
'''
FIELDS = ('command_ns', 'snapshot_ns', 'layout_ns', 'render_ns', 'key_to_render_ns')


def records(text, kind):
    for line in text.split('\n'):
        if line.startswith('{') and kind in line:
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            if record.get('diagnostic') == kind:
                yield record


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


def summary(keys):
    out = {}
    for field in FIELDS:
        values = [k[field] for k in keys if k.get(field) is not None]
        if values:
            out[field.removesuffix('_ns') + '_ms'] = {'p50': round(statistics.median(values), 1),
                                                      'p95': round(percentile(values, .95), 1),
                                                      'max': round(max(values), 1)}
    return out


def sizes(path):
    return path.read_text().split('\n')[:-1] if path.exists() else []


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--binary', required=True, type=Path)
    ap.add_argument('--output', required=True, type=Path)
    ap.add_argument('--files', type=int, default=5000)
    ap.add_argument('--input', default='file-1')
    ap.add_argument('--cycles', type=int, default=3)
    ap.add_argument('--font-size', default='15')
    ap.add_argument('--mode', default='2560x1440')
    ap.add_argument('--timeout', type=float, default=60)
    ap.add_argument('--assert-stable', action='store_true')
    ap.add_argument('--init', type=Path)
    ap.add_argument('--expect-styles')
    ap.add_argument('--labwc', default=live.LABWC)
    ap.add_argument('--wlr-randr', default=live.RANDR)
    args = ap.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    binary = args.binary.resolve()
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'files': args.files, 'input': args.input, 'cycles': args.cycles, 'keys': []}
    comp = kb = proc = None
    try:
        with tempfile.TemporaryDirectory(prefix='cpk-', dir=os.environ.get('XDG_RUNTIME_DIR')) as tmp:
            root = Path(tmp)
            root.chmod(0o700)
            home, work, config = root / 'home', root / 'work', root / 'config'
            for d in (home, work, config):
                d.mkdir()
            for n in range(args.files):
                (work / f'file-{n:04}.txt').write_text('x\n')
            (config / 'rc.xml').write_text(live.RC_XML)
            probe = root / 'pty-sizes'
            styles = root / 'styles'
            init = args.init.read_text() if args.init else ''
            (home / 'config' / 'romacs').mkdir(parents=True)
            (home / 'config' / 'romacs' / 'init.el').write_text(
                init + PROBE % {'path': json.dumps(str(styles))})
            env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
                   'ROMACS_GUI_FONT': 'Iosevka', 'ROMACS_GUI_FONT_SIZE': args.font_size, 'SHELL': '/bin/bash'}
            for k in ('CARGO_HOME', 'ROMACS_RUNTIME_ROOT', 'ROMACS_LISP_DIR', 'ROMACS_WORLD',
                      'ROMACS_PRESENT_TRACE', 'ROMACS_DISABLE_RETAINED_ROOT'):
                if k in os.environ:
                    env[k] = os.environ[k]
            log_path = out / 'binary.stderr'
            with (out / 'compositor.log').open('w') as clog, log_path.open('w') as log:
                comp = subprocess.Popen([args.labwc, '-C', str(config)], env=env, stdout=clog, stderr=clog)
                wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'compositor', comp)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                subprocess.run([args.wlr_randr, '--output', 'HEADLESS-1', '--custom-mode', args.mode],
                               env=env, check=True, timeout=15)
                kb = subprocess.Popen(['/usr/bin/wtype', '-s', '600000', '-k', 'Shift_L'], env=env,
                                      stdout=log, stderr=log)
                time.sleep(.3)
                proc = subprocess.Popen([str(binary), '--loadup', '--trace-latency'], env=env, cwd=work,
                                        stdin=subprocess.DEVNULL, stdout=log, stderr=log)
                trace = lambda: log_path.read_text(errors='replace')

                def keys(*parts, delay=40):
                    subprocess.run(['/usr/bin/wtype', '-d', str(delay), *parts], env=env, check=True, timeout=30)

                def ctrl(k):
                    keys('-M', 'ctrl', k, '-m', 'ctrl')

                def latencies():
                    return list(records(trace(), 'prompt-key-latency'))

                # A private HOME prepares its world on first start.
                wait(lambda: any(records(trace(), 'paint')), max(args.timeout, 600), 'first frame', proc)
                time.sleep(2)
                keys('-M', 'alt', 'x', '-m', 'alt')
                time.sleep(.5)
                keys('romacs-term', '-k', 'Return')
                time.sleep(2)
                keys(f"trap 'stty size >> {probe}' WINCH; stty size >> {probe}", '-k', 'Return')
                wait(lambda: sizes(probe), args.timeout, 'shell probe', proc)
                time.sleep(.5)
                created = sizes(probe)
                for cycle in range(args.cycles):
                    ctrl('x')
                    ctrl('f')
                    time.sleep(1)
                    for ch in args.input:
                        seen = len(latencies())
                        # wtype reads a leading '-' as an option.
                        keys(*(('-k', KEYSYMS[ch]) if ch in KEYSYMS else (ch,)), delay=10)
                        wait(lambda: len(latencies()) > seen, args.timeout, f'latency record for {ch!r}', proc)
                        time.sleep(.3)
                        record = latencies()[-1]
                        entry = {'cycle': cycle, 'key': ch}
                        entry.update({f: round(record[f] / 1e6, 2) for f in FIELDS if f in record})
                        report['keys'].append(entry)
                    ctrl('g')
                    time.sleep(1.5)
                # An editor command in the terminal buffer (C-g there is
                # romacs-term-keyboard-quit), which once resized the pty.
                ctrl('g')
                time.sleep(1.5)
                before_final = len(sizes(probe))
                keys(f'stty size >> {probe}', '-k', 'Return')
                wait(lambda: len(sizes(probe)) > before_final, args.timeout, 'final pty size', proc)
                lines = sizes(probe)
                report['pty'] = {'created': created[0], 'final': lines[-1],
                                 'winch': lines[len(created):-1], 'all': lines}
                report['summary'] = summary(report['keys'])
                report['completion'] = styles.read_text() if styles.exists() else None
                stable = report['pty']['created'] == report['pty']['final'] and not report['pty']['winch']
                report['pty']['stable'] = stable
                expected = args.expect_styles is None or (report['completion'] or '').startswith(
                    f"(:styles ({args.expect_styles})")
                report['status'] = 'PASS' if expected and (stable or not args.assert_stable) else 'FAIL'
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
        (out / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
        print(json.dumps({k: v for k, v in report.items() if k != 'keys'}, ensure_ascii=False))
    return 0 if report.get('status') == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
