#!/usr/bin/env python3
"""Capture one live GUI frame of a declared scene under a private compositor.

The oracle never sees anything this script learns from the editor.  The
editor is told to build the scene described by a spec file; when it says it
is ready, the compositor's own screenshot tool (grim) captures the output.
The capture is the only artifact the checker reads.

Isolation: a private labwc (WLR_BACKENDS=headless, pixman renderer) with its
own XDG_RUNTIME_DIR, a throwaway HOME and XDG tree for the editor, and
WAYLAND_DISPLAY pointing only at the private socket.  The ambient session is
never inherited.

usage: capture.py --editor gnu|romacs --spec SPEC.json --out OUT.png
                  [--romacs BIN] [--runtime-root DIR] [--keep]
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

LABWC = os.environ.get("ROMACS_TEST_LABWC", "labwc")
WLR_RANDR = os.environ.get("ROMACS_TEST_WLR_RANDR", "wlr-randr")
GNU = '/usr/bin/emacs'
GRIM = '/usr/bin/grim'
WTYPE = '/usr/bin/wtype'
# Unix socket paths cap at 108 bytes, so the private runtime lives under a
# short root rather than the scratch directory.
ROOT = Path(tempfile.gettempdir())

RC_XML = """<?xml version="1.0"?>
<labwc_config>
  <core><decoration>server</decoration><gap>0</gap></core>
  <windowRules>
    <windowRule identifier="*" serverDecoration="no">
      <action name="Maximize"/>
    </windowRule>
  </windowRules>
</labwc_config>
"""


def elisp_string(s):
    return '"' + s.replace('\\', '\\\\').replace('"', '\\"') + '"'


def face_plist(seg):
    attrs = []
    if 'fg' in seg:
        attrs += [':foreground', elisp_string(seg['fg'])]
    if 'bg' in seg:
        attrs += [':background', elisp_string(seg['bg'])]
    if seg.get('bold'):
        attrs += [':weight', 'bold']
    if seg.get('italic'):
        attrs += [':slant', 'italic']
    if seg.get('underline'):
        attrs += [':underline', 't']
    return '(' + ' '.join(attrs) + ')' if attrs else None


def scene_el(spec, ready):
    """Lisp that builds the scene.  Uses only GNU-documented primitives."""
    f = spec['frame']
    font = spec['font']
    lines = [
        ';;; -*- lexical-binding: t -*-',
        '(setq inhibit-startup-screen t initial-scratch-message nil)',
        '(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)',
        '(blink-cursor-mode -1)',
        '(setq-default cursor-in-non-selected-windows nil)',
        '(set-fringe-mode %d)' % f.get('fringe', 0),
        "(modify-frame-parameters nil '((internal-border-width . 0)))",
        "(set-face-attribute 'default nil :font %s)" % elisp_string(
            '%s:pixelsize=%d' % (font['family'], font['pixel_size'])),
        "(set-face-attribute 'default nil :foreground %s :background %s)" % (
            elisp_string(f['fg']), elisp_string(f['bg'])),
        "(set-face-attribute 'cursor nil :background %s)" % elisp_string(spec['cursor_color']),
        "(set-face-attribute 'fringe nil :background %s :foreground %s)" % (
            elisp_string(f.get('fringe_bg', f['bg'])), elisp_string(f.get('fringe_fg', f['fg']))),
        "(setq-default mode-line-format nil)",
        "(setq-default truncate-lines nil)",
        "(setq-default line-spacing nil)",
        "(global-hl-line-mode -1)",
        '(switch-to-buffer (get-buffer-create "oracle-scene"))',
        '(fundamental-mode)',
        '(setq mode-line-format nil)',
        '(delete-other-windows)',
        '(erase-buffer)',
    ]
    for seg in spec['buffer']:
        plist = face_plist(seg)
        if plist:
            lines.append('(insert (propertize %s (quote face) (quote %s)))' % (
                elisp_string(seg['text']), plist))
        else:
            lines.append('(insert %s)' % elisp_string(seg['text']))
    lines += [
        '(goto-char %d)' % spec.get('point', 1),
        '(set-buffer-modified-p nil)',
        '(message nil)',
        '(run-with-timer 1.5 nil (lambda () (message nil) (redisplay t)'
        ' (with-temp-file %s (insert "ready"))))' % elisp_string(str(ready)),
    ]
    return '\n'.join(lines) + '\n'


def wait_for(pred, timeout, what, proc=None):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            return
        if proc is not None and proc.poll() is not None:
            raise RuntimeError(f'{what}: process exited {proc.returncode}')
        time.sleep(0.05)
    raise RuntimeError(f'timeout waiting for {what}')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--editor', choices=['gnu', 'romacs'], required=True)
    ap.add_argument('--spec', required=True)
    ap.add_argument('--out', required=True)
    ap.add_argument('--romacs')
    ap.add_argument('--runtime-root')
    ap.add_argument('--settle', type=float, default=0.7)
    ap.add_argument('--keep', action='store_true')
    a = ap.parse_args()
    spec = json.loads(Path(a.spec).read_text())
    ROOT.mkdir(parents=True, exist_ok=True)
    d = Path(tempfile.mkdtemp(prefix='c', dir=ROOT))
    run = d / 'r'
    run.mkdir(mode=0o700)
    (d / 'lw').mkdir()
    (d / 'lw' / 'rc.xml').write_text(RC_XML)
    cenv = {'PATH': '/usr/bin:/bin', 'HOME': str(d), 'XDG_RUNTIME_DIR': str(run),
            'WLR_BACKENDS': 'headless', 'WLR_HEADLESS_OUTPUTS': '1',
            'WLR_LIBINPUT_NO_DEVICES': '1', 'WLR_RENDERER': 'pixman'}
    comp_log = (d / 'labwc.log').open('w')
    comp = subprocess.Popen([LABWC, '-C', str(d / 'lw')], env=cenv,
                            stdout=comp_log, stderr=subprocess.STDOUT)
    ed = None
    kbd = None
    try:
        wait_for(lambda: (run / 'wayland-0').is_socket(), 10, 'compositor socket', comp)
        wenv = dict(cenv, WAYLAND_DISPLAY='wayland-0')
        w, h = spec['frame']['width'], spec['frame']['height']
        subprocess.run([WLR_RANDR, '--output', 'HEADLESS-1', '--custom-mode', f'{w}x{h}'],
                       env=wenv, check=True, timeout=10, stdout=subprocess.DEVNULL)
        h_ed = d / 'h'
        for sub in ('c', 'ca', 'd', 's', 'tmp'):
            (h_ed / sub).mkdir(parents=True)
        ready = d / 'ready'
        el = d / 'scene.el'
        el.write_text(scene_el(spec, ready))
        eenv = {'PATH': '/usr/bin:/bin', 'HOME': str(h_ed),
                'XDG_CONFIG_HOME': str(h_ed / 'c'), 'XDG_CACHE_HOME': str(h_ed / 'ca'),
                'XDG_DATA_HOME': str(h_ed / 'd'), 'XDG_STATE_HOME': str(h_ed / 's'),
                'TMPDIR': str(h_ed / 'tmp'), 'LANG': 'C.UTF-8',
                'XDG_RUNTIME_DIR': str(run), 'WAYLAND_DISPLAY': 'wayland-0',
                'GDK_BACKEND': 'wayland', 'GSETTINGS_BACKEND': 'memory',
                'NO_AT_BRIDGE': '1'}
        if a.editor == 'gnu':
            cmd = [GNU, '-Q', '-l', str(el)]
        else:
            if not a.romacs or not a.runtime_root:
                raise SystemExit('--romacs and --runtime-root are required for romacs')
            eenv['ROMACS_RUNTIME_ROOT'] = a.runtime_root
            cmd = [a.romacs, '-Q', '-l', str(el)]
        # A virtual keyboard must exist on the seat before the editor maps,
        # or the frame never receives keyboard focus and draws an unfocused
        # cursor.  wtype holds one for as long as it sleeps; it is killed
        # after the capture, before it would type anything.
        kbd = subprocess.Popen([WTYPE, '-s', '120000', '-k', 'Shift_L'], env=wenv)
        time.sleep(0.3)
        ed_log = (d / 'editor.log').open('w')
        ed = subprocess.Popen(cmd, env=eenv, stdout=ed_log, stderr=subprocess.STDOUT,
                              stdin=subprocess.DEVNULL)
        wait_for(ready.exists, 60, 'editor ready', ed)
        time.sleep(a.settle)
        if ed.poll() is not None:
            raise RuntimeError(f'editor exited {ed.returncode} before capture')
        out = Path(a.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run([GRIM, str(out)], env=wenv, check=True, timeout=10)
        print(json.dumps({'capture': str(out), 'editor': a.editor, 'workdir': str(d),
                          'cmd': cmd}))
    finally:
        if kbd is not None:
            kbd.kill()
            kbd.wait()
        if ed is not None and ed.poll() is None:
            ed.terminate()
            try:
                ed.wait(timeout=5)
            except subprocess.TimeoutExpired:
                ed.kill()
                ed.wait()
        comp.terminate()
        try:
            comp.wait(timeout=5)
        except subprocess.TimeoutExpired:
            comp.kill()
            comp.wait()
        if not a.keep:
            shutil.rmtree(d, ignore_errors=True)


if __name__ == '__main__':
    sys.exit(main())
