#!/usr/bin/env python3
"""Record pinned GNU's startup after the init file, on a window system.

GNU runs no startup hooks in batch (`command-line' ends in `kill-emacs'),
and file arguments, `-Q' and the startup screen act differently on a
terminal, so this runs the pinned pgtk GNU in a private headless labwc
(the gate's headless tools) with a private HOME whose ~/.emacs.d/init.el
is each scenario's INIT, and the scenario's command line.  A `-Q' scenario
loads no init: its INIT comes in through `-l', which `command-line-1'
processes before it decides on the startup screen.

Each INIT starts with PROBE, which logs (HOOK SELECTED-BUFFER
WINDOW-BUFFERS ROMACS-PROBE-FRAME-PARAMETER) at the startup hooks and
writes `startup-order-record' from an idle timer once startup is over.
`startup-order-screen' names the startup screen's buffer: GNU's
"*GNU Emacs*", which startup.rs's tests set to Romacs's "*Romacs*".
A scenario with KEYS records the screen's bindings of them, then types
each through the command loop (`unread-command-events', from a timer),
noting after each the selected buffer and whether the screen lives.

Writes startup-order-gui.txt beside this script: per scenario its name,
its arguments, its KEYS (character codes), INIT, and GNU's record.

usage: startup-order-gui.py EMACS
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

TOOLS = os.environ.get('ROMACS_HEADLESS_TOOLS', '/nix/store/klrgwbvpwd7napzdhacw1nhj8xmpbpdc-romacs-gate-headless-tools/bin')
RC_XML = '<?xml version="1.0"?><labwc_config><core><gap>0</gap></core></labwc_config>\n'

PROBE = ('(defvar startup-order-screen "*GNU Emacs*") (defvar startup-order-log nil)'
         ' (defun startup-order-windows () (mapcar (lambda (w) (buffer-name (window-buffer w))) (window-list nil (quote no-mini) (frame-first-window))))'
         ' (defun startup-order-note (hook) (push (list hook (buffer-name (window-buffer (selected-window))) (startup-order-windows) (frame-parameter nil (quote romacs-probe))) startup-order-log))'
         ' (add-hook (quote emacs-startup-hook) (lambda () (startup-order-note (quote emacs-startup))))'
         ' (add-hook (quote window-setup-hook) (lambda () (startup-order-note (quote window-setup))))'
         ' (defun startup-order-record () (list (reverse startup-order-log) (buffer-name (window-buffer (selected-window))) (startup-order-windows)'
         ' (if (get-buffer "*scratch*") (with-current-buffer "*scratch*" (list (buffer-substring-no-properties (point-min) (point-max)) (buffer-modified-p) major-mode)) (quote none))'
         ' (and (get-buffer startup-order-screen) t) (frame-parameter nil (quote romacs-probe)) inhibit-x-resources'
         ' (and (get-buffer startup-order-screen) (file-relative-name (with-current-buffer startup-order-screen default-directory) (getenv "HOME")))))'
         ' (defvar startup-order-keys nil)'
         ' (defun startup-order-bindings (keys) (with-current-buffer startup-order-screen (mapcar (lambda (k) (key-binding (vector k))) keys)))'
         ' (defun startup-order-key-note (key) (push (list key (buffer-name (window-buffer (selected-window))) (and (get-buffer startup-order-screen) t)) startup-order-keys))'
         ' (defun startup-order-key-record (bindings) (list (startup-order-record) bindings (reverse startup-order-keys)))')
AFTER_INIT = ' (add-hook (quote after-init-hook) (lambda () (startup-order-note (quote after-init))))'
# Each scenario: (NAME, ARGS, INIT), or (NAME, ARGS, INIT, KEYS).
SCENARIOS = [
    # An after-init hook that switches buffers: the file argument is
    # visited after it; the concise startup screen shows beside it.
    ('hook-then-file', 'visited.txt',
     PROBE + ' (add-hook (quote after-init-hook) (lambda () (startup-order-note (quote after-init))'
             ' (switch-to-buffer (get-buffer-create "*hooked*"))))'),
    # The startup screen is its own buffer: *scratch* keeps the init's text.
    ('own-scratch-screen', '',
     PROBE + AFTER_INIT + ' (with-current-buffer "*scratch*" (erase-buffer) (insert "mine\\n"))'),
    # An inhibiting init keeps its *scratch* text too.
    ('own-scratch-inhibited', '',
     PROBE + AFTER_INIT + ' (setq inhibit-startup-screen t) (with-current-buffer "*scratch*" (erase-buffer) (insert "mine\\n"))'),
    # initial-buffer-choice: a function's buffer is selected.
    ('choice-function', '',
     PROBE + AFTER_INIT + ' (setq initial-buffer-choice (lambda () (get-buffer-create "*chosen*")))'),
    # t recreates and selects *scratch*, with the initial message.
    ('choice-t', '',
     PROBE + AFTER_INIT + ' (kill-buffer "*scratch*") (setq initial-buffer-choice t)'),
    # A file name is visited and shown first, beside the file argument.
    ('choice-string-and-file', 'visited.txt',
     PROBE + AFTER_INIT + ' (setq initial-buffer-choice "chosen.txt")'),
    # -Q loads no init; the probe comes in through -l.
    ('quick', '-Q -l probe.el', PROBE),
    # The init's frame parameters apply between term-setup-hook and
    # window-setup-hook, on the screen's path and on the inhibited one.
    ('frame-screen', '', PROBE + AFTER_INIT + ' (push (quote (romacs-probe . 42)) initial-frame-alist)'),
    ('frame-inhibited', '', PROBE + AFTER_INIT + ' (setq inhibit-startup-screen t) (push (quote (romacs-probe . 42)) initial-frame-alist)'),
    # An emacs-startup-hook that inhibits the startup hooks stops
    # window-setup-hook (normal-top-level checks again).
    ('startup-hook-inhibits', '',
     PROBE + AFTER_INIT + ' (setq inhibit-startup-screen t) (add-hook (quote emacs-startup-hook) (lambda () (setq inhibit-startup-hooks t)))'),
    # The screen's directory is the command line's, not the visited file's.
    ('screen-directory', 'sub/visited.txt', PROBE + AFTER_INIT),
    # The screen's keymap: typing does not dismiss it, SPC and DEL scroll,
    # q kills it and shows *scratch* again.
    ('screen-keys', '', PROBE + AFTER_INIT, '97 32 127 113'),
]
WRITER = (' (run-with-idle-timer 1 nil (lambda () (with-temp-file {out!r} (let ((print-escape-newlines t))'
          ' (prin1 (startup-order-record) (current-buffer)))) (kill-emacs 0)))\n')
# Types KEYS one per half second once startup is over, noting each.
KEYS_WRITER = (' (defun startup-order-type (keys bindings) (if keys (progn (setq unread-command-events (list (car keys)))'
               ' (run-with-timer 0.5 nil (lambda () (startup-order-key-note (car keys)) (startup-order-type (cdr keys) bindings))))'
               ' (with-temp-file {out!r} (let ((print-escape-newlines t)) (prin1 (startup-order-key-record bindings) (current-buffer))))'
               ' (kill-emacs 0)))'
               ' (run-with-idle-timer 1 nil (lambda () (startup-order-type (quote ({keys})) (startup-order-bindings (quote ({keys}))))))\n')


def wait(predicate, seconds, what):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        found = predicate()
        if found:
            return found
        time.sleep(.05)
    raise SystemExit(f'{what} did not happen')


def record(emacs, args, init, keys):
    with tempfile.TemporaryDirectory(prefix='startup-order-') as temporary:
        root = Path(temporary)
        home = root / 'home'
        (home / '.emacs.d').mkdir(parents=True)
        (root / 'labwc').mkdir()
        (root / 'labwc/rc.xml').write_text(RC_XML)
        out = root / 'answer.el'
        # A file without the cookie makes GNU warn in a window of its own.
        writer = KEYS_WRITER.format(out=str(out), keys=keys) if keys else WRITER.format(out=str(out))
        source = ';; -*- lexical-binding: t -*-\n' + (init + writer).replace("'", '"')
        (home / ('probe.el' if '-Q' in args else '.emacs.d/init.el')).write_text(source)
        (home / 'visited.txt').write_text('visited\n')
        (home / 'sub').mkdir()
        (home / 'sub/visited.txt').write_text('visited\n')
        (home / 'chosen.txt').write_text('chosen\n')
        # The harness's headless compositor environment.
        env = {'PATH': f'{TOOLS}:{os.environ["PATH"]}', 'HOME': str(home), 'LANG': 'C.UTF-8', 'XDG_RUNTIME_DIR': str(root),
               'XDG_CONFIG_HOME': str(home / 'c'), 'XDG_CACHE_HOME': str(home / 'k'), 'XDG_DATA_HOME': str(home / 'd'),
               'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
               'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop', 'GSETTINGS_BACKEND': 'memory',
               'DBUS_SESSION_BUS_ADDRESS': 'unix:path=/nonexistent', 'NO_AT_BRIDGE': '1', 'GDK_BACKEND': 'wayland'}
        compositor = subprocess.Popen([f'{TOOLS}/labwc', '-C', str(root / 'labwc')], env=env,
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            socket = wait(lambda: next((p for p in root.glob('wayland-*') if p.is_socket()), None), 15, 'compositor')
            env['WAYLAND_DISPLAY'] = socket.name
            editor = subprocess.Popen([emacs, '--no-site-file'] + args.split(), env=env, cwd=home,
                                      stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                editor.wait(timeout=60)
            except subprocess.TimeoutExpired:
                editor.kill()
                raise SystemExit('GNU did not finish')
        finally:
            compositor.kill()
            compositor.wait()
        return out.read_text()


def main():
    emacs = sys.argv[1]
    lines = [';; Pinned GNU (pgtk, headless labwc), by startup-order-gui.py; do not edit.']
    for name, args, init, *keys in SCENARIOS:
        keys = keys[0] if keys else ''
        lines += [name, args, keys, init, record(emacs, args, init, keys)]
    (Path(__file__).resolve().parent / 'startup-order-gui.txt').write_text('\n'.join(lines) + '\n')


if __name__ == '__main__':
    main()
