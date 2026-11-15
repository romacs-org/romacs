#!/usr/bin/env python3
"""A user's own versions of bundled Lisp win over the bundled ones in every
startup path a --loadup session takes, as GNU's load-path shadowing does.

A private HOME's init puts ~/my-lisp first on `load-path' and requires two
libraries a session does not load at startup, a Romacs one (romacs-agent-diff)
and a GNU one (hexl), each copied there from the installation with a marker
added; it redefines one hexl function, advises another, and writes what it
finds to a state file.  First a session whose init requires the bundled
copies loads them, so the compiled-Lisp store holds their artifacts.  Then,
with the user's init: a source boot (ROMACS_WORLD=off); a session restored
from the base world, which records the init's layer and builds it apart; a
warm one restored from that layer; and one with the layer's plan gone, which
loads the libraries again through the compiled-Lisp store.  In each, the
user's copies, redefinition and advice must be the ones running, and the
store's use log must show no bundled artifact of a shadowed library used
after the bundled session.  Libraries a session loads before the init (one the
base world preloads, thingatpt; one the defaults turn on, romacs-cursor) are
reported, not checked: `require' finds them loaded and reads no copy, as GNU
does for what it loads before an init.

Every process gets a private HOME/XDG tree and the private compositor's
WAYLAND_DISPLAY.  labwc and wlr-randr come from --tools or PATH.
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

SHADOWED = (('romacs-agent-diff', 'lisp/romacs/romacs-agent-diff.el'), ('hexl', 'lisp/gnu/hexl.el'))
PRELOADED = (('thingatpt', 'lisp/gnu/thingatpt.el'), ('romacs-cursor', 'lisp/romacs/romacs-cursor.el'))

STATE = '''(write-region
 (prin1-to-string
  (list :diff-file (symbol-file 'romacs-agent-diff 'provide)
        :diff-marker (and (boundp 'romacs-shadowing-romacs-agent-diff) romacs-shadowing-romacs-agent-diff)
        :hexl-file (symbol-file 'hexl 'provide)
        :hexl-marker (and (boundp 'romacs-shadowing-hexl) romacs-shadowing-hexl)
        :redefined (and (fboundp 'hexl-hex-char-to-integer) (hexl-hex-char-to-integer ?a))
        :advised (and (fboundp 'hexl-printable-character) (hexl-printable-character ?a))
        :thingatpt-file (symbol-file 'thingatpt 'provide)
        :thingatpt-marker (and (boundp 'romacs-shadowing-thingatpt) romacs-shadowing-thingatpt)
        :cursor-file (symbol-file 'romacs-cursor 'provide)
        :cursor-marker (and (boundp 'romacs-shadowing-romacs-cursor) romacs-shadowing-romacs-cursor)))
 nil {state})
'''

BUNDLED_INIT = ''';; -*- lexical-binding: t -*-
(require 'hexl)
(require 'romacs-agent-diff)
''' + STATE

USER_INIT = ''';; -*- lexical-binding: t -*-
(add-to-list 'load-path (expand-file-name "my-lisp" "~"))
;; hexl first: romacs-agent runs git as it loads, so its entry, and every
;; startup entry after it, runs live rather than from the layer.
(require 'hexl)
(require 'romacs-agent-diff)
(require 'thingatpt)
(require 'romacs-cursor)
(defun hexl-hex-char-to-integer (_character) 'user-redefined)
(advice-add 'hexl-printable-character :override (lambda (&rest _) 'user-advised))
''' + STATE


def user_copy(source, feature):
    """SOURCE's text with a marker defined before its `provide'."""
    text = source.read_text()
    provide = f"(provide '{feature})"
    assert provide in text, f'{source} provides no {feature}'
    return text.replace(provide, f"(defvar romacs-shadowing-{feature} 'user)\n{provide}", 1)


def running(pattern):
    """Whether a process whose command line holds PATTERN runs."""
    return bool(subprocess.run(['pgrep', '-f', '--', pattern], capture_output=True, text=True).stdout.split())


def settle(patterns, limit):
    """Until no process matches any of PATTERNS, three checks a second apart."""
    deadline, quiet = time.monotonic() + limit, 0
    while time.monotonic() < deadline and quiet < 3:
        quiet = 0 if any(running(p) for p in patterns) else quiet + 1
        time.sleep(1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--runtime', help='the Romacs tree holding lisp/ (default: the binary\'s own)')
    parser.add_argument('--tools', help='directory with labwc and wlr-randr')
    parser.add_argument('--timeout', type=float, default=300)
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    runtime = Path(args.runtime or os.environ.get('ROMACS_RUNTIME_ROOT') or Path(__file__).resolve().parents[3])
    report = {'binary': str(binary), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'status': 'FAIL', 'launches': [], 'checks': [], 'failures': [], 'pinned': []}
    compositor = None
    try:
        with tempfile.TemporaryDirectory(prefix='user-shadowing-') as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            home, work, config = [root / name for name in ('home', 'work', 'config')]
            for directory in (home, work, config):
                directory.mkdir()
            (config / 'rc.xml').write_text(live.RC_XML)
            path = os.environ.get('PATH', '/usr/bin:/bin')
            env = {'PATH': f'{args.tools}:{path}' if args.tools else path,
                   'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
                   'ROMACS_RUNTIME_ROOT': str(runtime),
                   'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
                   'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
                   'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
                   'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
                   'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop'}
            for key in ('ROMACS_LISP_DIR', 'ROMACS_GATE_FONTCONFIG'):
                if key in os.environ:
                    env[key] = os.environ[key]
            if 'ROMACS_GATE_FONTCONFIG' in env:
                env['FONTCONFIG_FILE'] = env['ROMACS_GATE_FONTCONFIG']
            store = home / 'cache/romacs/worlds'
            init_dir = home / 'config/romacs'
            init_dir.mkdir(parents=True)
            my_lisp = home / 'my-lisp'
            my_lisp.mkdir()
            for feature, source in (*SHADOWED, *PRELOADED):
                (my_lisp / Path(source).name).write_text(user_copy(runtime / source, feature))

            def check(name, ok, detail):
                report['checks'].append({'check': name, 'ok': bool(ok), 'detail': detail})
                if not ok:
                    report['failures'].append(name)

            def uses(name):
                """The compiled-Lisp store's use-log lines naming NAME's file."""
                lines = []
                for log in Path(f'{binary}.lisp').glob('*/use.log'):
                    lines += [line for line in log.read_text(errors='replace').splitlines() if line.endswith(f'/{name}')]
                return lines

            with (output / 'compositor.log').open('w') as compositor_log:
                compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or live.LABWC, '-C', str(config)],
                                              env=env, stdout=compositor_log, stderr=compositor_log)
                fio.wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'private compositor', compositor)
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                subprocess.run(['wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1280x800'],
                               env=env, check=True, timeout=15)

                def launch(name, init, extra_env=None, linger=0):
                    """Run --loadup with INIT to its first frame and its state
                    file, linger, end it, and let the builders and compiles it
                    started finish: how it booted and what its init found."""
                    # One state file for every launch, outside HOME: the init's
                    # content and the listing of HOME, the parent of a
                    # load-path directory, key its layer.
                    state = output / 'init.state'
                    state.unlink(missing_ok=True)
                    (init_dir / 'init.el').write_text(init.format(state=json.dumps(str(state))))
                    log_path = output / f'{name}.stderr'
                    with log_path.open('w') as log:
                        process = subprocess.Popen([str(binary), '--loadup', '--trace-scene'], env={**env, **(extra_env or {})}, cwd=work,
                                                   stdin=subprocess.DEVNULL, stdout=log, stderr=log)
                        try:
                            fio.wait(lambda: ff.current_rows(log_path.read_text(errors='replace')),
                                     args.timeout, f'{name}: first frame', process)
                            fio.wait(state.exists, args.timeout, f'{name}: init state', process)
                            time.sleep(linger)
                        finally:
                            process.terminate()
                            try:
                                process.wait(timeout=30)
                            except subprocess.TimeoutExpired:
                                process.kill()
                                process.wait()
                    settle([f'--world-store {store}', '--compile-lisp'], args.timeout)
                    text = log_path.read_text(errors='replace')
                    boot = [line.split(': ', 1)[1] for line in text.splitlines() if line.startswith('romacs-world: {') and '"mode"' in line]
                    found = state.read_text()
                    (output / f'{name}.state').write_text(found)
                    entry = {'launch': name, 'boot': boot, 'state': found}
                    report['launches'].append(entry)
                    return entry

                def user_versions(entry):
                    found = entry['state']
                    return (f'{my_lisp}/romacs-agent-diff.el' in found and f'{my_lisp}/hexl.el' in found
                            and ':diff-marker user' in found and ':hexl-marker user' in found
                            and ':redefined user-redefined' in found and ':advised user-advised' in found)

                bundled = launch('bundled', BUNDLED_INIT, linger=20)
                check('the bundled session loads the bundled copies',
                      'my-lisp' not in bundled['state'] and ':hexl-marker nil' in bundled['state'], bundled)
                bundled_uses = {name: len(uses(Path(source).name)) for name, source in SHADOWED}
                source = launch('source', USER_INIT, {'ROMACS_WORLD': 'off'})
                check("a source boot runs the user's copies, redefinition and advice", user_versions(source), source)
                base = launch('base', USER_INIT)
                check("a session restored from the base world runs the user's copies, redefinition and advice",
                      user_versions(base), base)
                layer = launch('layer', USER_INIT)
                report['layer_builds'] = []
                for manifest in sorted(store.glob('*.json')):
                    data = json.loads(manifest.read_text())
                    if 'init-layer' in json.dumps(data.get('events', {}))[:200] or data.get('uncaptured') or data.get('refusals'):
                        report['layer_builds'].append({'manifest': manifest.name[:16],
                                                       'entries': [e.get('name') for e in data.get('events', {}).get('init-layer', {}).get('entries', [])],
                                                       'uncaptured': (data.get('uncaptured') or data.get('refusals') or [])[:20]})
                check('the warm session restores its init layer', any('"layer"' in b for b in layer['boot']), layer['boot'])
                check("a session restored from the init layer runs the user's copies, redefinition and advice",
                      user_versions(layer), layer)
                plans = store / 'plans'
                if plans.exists():
                    shutil.rmtree(plans)
                compiled = launch('compiled', USER_INIT, linger=10)
                check("a session loading through the compiled-Lisp store runs the user's copies, redefinition and advice",
                      user_versions(compiled), compiled)
                later_uses = {name: len(uses(Path(source).name)) for name, source in SHADOWED}
                bundled_paths = {name: [line for line in uses(Path(src).name) if str(my_lisp) not in line][bundled_uses[name]:]
                                 for name, src in SHADOWED}
                check('no bundled artifact of a shadowed library is used once the user copies shadow it',
                      not any(bundled_paths.values()), {'after': bundled_paths, 'uses': later_uses})
                for entry in (source, base, layer, compiled):
                    for feature, src in PRELOADED:
                        if f'{my_lisp}/{Path(src).name}' not in entry['state']:
                            report['pinned'].append({'launch': entry['launch'], 'library': feature,
                                                     'why': 'loaded before the init: require reads no copy'})
                report['status'] = 'PASS' if not report['failures'] else 'FAIL'
    finally:
        if compositor is not None:
            compositor.terminate()
            compositor.wait(timeout=15)
        (output / 'report.json').write_text(json.dumps(report, indent=2))
    print(report['status'], json.dumps({'failures': report['failures'], 'pinned': report['pinned']}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
