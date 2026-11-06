#!/usr/bin/env python3
"""Server-first launch (rlisp/romacs-server.el, src/client.rs) in a private
headless labwc, HOME, XDG and TMPDIR (parity_live.py).

  lifecycle  `romacs --server S FILE -n' with no server starts one (detached:
             the private tree has no user manager) and shows its window;
             a second client with the server up shows FILE2 within 300 ms;
             `emacsclient -s S' evaluates, visits `+3:4 FILE' at line 3
             column 4, and is refused a text-terminal frame; C-x C-c closes
             the window and keeps the session, and the next client shows it
             again and paints; `--stop=none' ends the server and its socket.
  waiters    a native client waiting on three files returns after the third
             is done with C-x # (finished out of order), and not before; one
             that disconnects first is forgotten; a buffer a native and an
             emacsclient client both wait on finishes both with one C-x #;
             killing a buffer finishes its native waiter; C-x # on a modified
             buffer asks GNU's save question and saves on y.
  unit       with the user manager reachable, the client starts the server
             as the transient unit romacs-server-S, which a closed window
             leaves active and `--stop' ends.
  crash      with the user manager: a server killed by SIGKILL shows a
             desktop notification (its unit's ExecStopPost; notify-send is a
             recording stand-in) naming the journal command; `--stop' and a
             unit stop (a logout's TERM) show none.
  build      the server keeps its build when the one ROMACS_BUILD_WATCH
             names changes: it says "Romacs <sha> installed; quit and
             relaunch to switch" once, at its next idle with the window
             shown, never while the window is hidden.
  probe      clients x files: finishing one buffer and retiring one client
             with many pending buffers, at 1, 10 and 100 in each dimension
             (the costs must not grow with the other dimension).

usage: server_live.py --romacs BIN --output DIR [--workflow NAME]... [--tools DIR]
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

import parity_live
from parity_live import prepare_world, private_tree, wait

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
'''


class Server:
    """A private tree with a compositor, in which clients start server NAME."""

    def __init__(self, args, base, name, env_extra=None):
        self.args, self.name = args, name
        self.root, self.home, self.env = private_tree('romacs', base, args.tools, prefix=f'server-{name}-')
        # Sockets need short paths (108 bytes): a runtime directory of their
        # own on the host's, while the tree (worlds, journals) stays on disk.
        host_runtime = Path(os.environ.get('XDG_RUNTIME_DIR') or f'/run/user/{os.getuid()}')
        self.runtime = Path(tempfile.mkdtemp(prefix='rs-', dir=host_runtime))
        self.env['XDG_RUNTIME_DIR'] = str(self.runtime)
        (self.home / '.config/romacs').mkdir()
        (self.home / '.config/romacs/init.el').write_text(INIT)
        self.env.update(ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14', **(env_extra or {}))
        # The real GPU, when asked for: the compositor stays private.
        wlr = dict(pair.split('=', 1) for pair in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split())
        if wlr:
            self.env.update(wlr)
            self.env.pop('WGPU_BACKEND', None)
        for key in ('ROMACS_GNU_SOURCE', 'ROMACS_RUNTIME_ROOT', 'ROMACS_LISP_DIR'):
            if os.environ.get(key):
                self.env[key] = os.environ[key]
        (self.root / 'labwc').mkdir()
        (self.root / 'labwc/rc.xml').write_text(parity_live.RC_XML)
        log = (self.root / 'compositor.log').open('w')
        self.compositor = subprocess.Popen([f'{args.tools}/labwc', '-C', str(self.root / 'labwc')],
                                           env=self.env, stdout=log, stderr=log)
        socket = wait(lambda: next((p for p in self.runtime.glob('wayland-*') if p.is_socket()), None),
                      15, 'compositor', self.compositor)
        self.env['WAYLAND_DISPLAY'] = socket.name
        subprocess.run([f'{args.tools}/wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1280x900'],
                       env=self.env, check=True, timeout=15, capture_output=True)
        # A virtual keyboard held open, as parity_live's engines do, so later
        # wtype keys reach the focused window.
        self.keyboard = subprocess.Popen([f'{args.tools}/wtype', '-s', '86400000', '-k', 'Shift_L'],
                                         env=self.env, stdout=log, stderr=log)
        self.world_seconds = prepare_world(args.romacs, self.env, self.root / 'work')
        self.pid = None
        self.unit = None

    def client(self, *argv, timeout=120, check=True):
        """Run `romacs --server NAME ARGV'; (seconds, stdout)."""
        started = time.time()
        # No launch mode: the launcher's default (scripts/romacs) gives none.
        done = subprocess.run([self.args.romacs, '--server', self.name, *argv], env=self.env,
                              cwd=self.root / 'work', capture_output=True, text=True, timeout=timeout)
        if check and done.returncode != 0:
            raise RuntimeError(f'client {argv}: {done.returncode} {done.stderr[-2000:]}')
        return time.time() - started, done.stdout.strip() if done.returncode == 0 else done.stderr.strip()

    def waiting(self, *argv):
        return subprocess.Popen([self.args.romacs, '--server', self.name, *argv], env=self.env,
                                cwd=self.root / 'work', stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)

    def eval(self, form):
        return self.client('-e', form)[1]

    def emacsclient(self, *argv, check=True):
        done = subprocess.run([self.args.emacsclient, '-s', self.name, *argv], env=self.env,
                              cwd=self.root / 'work', capture_output=True, text=True, timeout=60)
        if check and done.returncode != 0:
            raise RuntimeError(f'emacsclient {argv}: {done.returncode} {done.stderr}')
        return done

    def keys(self, *argv):
        subprocess.run([f'{self.args.tools}/wtype', *argv], env=self.env, check=True, timeout=15)

    def keyboard_ready(self):
        """Wait until keys reach the window: the compositor focuses a new
        window some time after its surface exists.  C-g until it runs."""
        def quit_ran():
            before = self.log().count('"native_command":"keyboard-quit"')
            self.keys('-M', 'ctrl', '-k', 'g', '-m', 'ctrl')
            time.sleep(.3)
            return self.log().count('"native_command":"keyboard-quit"') > before
        until(quit_ran, 30, 'keys reach the window')

    def file(self, name, text='one\ntwo\nthree three\nfour\n'):
        path = self.root / 'work' / name
        path.write_text(text)
        return path

    def log(self):
        if self.unit:
            return subprocess.run(['journalctl', '--user', '--no-pager', '-o', 'cat', '-u', self.unit],
                                  capture_output=True, text=True).stdout
        path = self.home / '.local/state/romacs' / f'server-{self.name}.log'
        return path.read_text(errors='replace') if path.exists() else ''

    def close(self):
        if self.pid:
            try:
                os.kill(self.pid, 9)
            except ProcessLookupError:
                pass
        self.keyboard.kill()
        self.compositor.kill()
        self.compositor.wait()
        shutil.rmtree(self.runtime, ignore_errors=True)


def until(predicate, seconds, what):
    end = time.time() + seconds
    while time.time() < end:
        value = predicate()
        if value:
            return value
        time.sleep(.05)
    raise RuntimeError(f'timed out: {what}')


def lisp(path):
    return parity_live.lisp_string(str(path))


def standalone_launch(s, path):
    """Seconds to a standalone `--loadup FILE' session's first frame drawn
    (its `ready' line), in the same tree: today's warm launch."""
    started = time.time()
    process = subprocess.Popen([s.args.romacs, '--loadup', str(path)], env=s.env, cwd=s.root / 'work',
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        for line in process.stdout:
            if '"ready":true' in line:
                return round(time.time() - started, 3)
        raise RuntimeError('standalone session ended before its first frame')
    finally:
        process.kill()
        process.wait()


def lifecycle(args, base):
    s = Server(args, base, 'lc')
    try:
        a, b = s.file('a.txt'), s.file('b.txt')
        # Today's launch and a server's first, paired and alternated (both
        # meet the same caches and load); medians of three.
        pairs = []
        for _ in range(3):
            alone = standalone_launch(s, a)
            first, _ = s.client('-n', str(a))
            pid = int(s.eval('(emacs-pid)'))
            s.client('--stop=none')
            until(lambda: not Path(f'/proc/{pid}').exists(), 15, 'server stopped between pairs')
            pairs.append((alone, first))
        standalone = sorted(p[0] for p in pairs)[1]
        cold = sorted(p[1] for p in pairs)[1]
        s.client('-n', str(a))
        s.pid = int(s.eval('(emacs-pid)'))
        shown = s.eval("(romacs--native-window 'query)")
        warm, _ = s.client('-n', str(b))
        current = s.eval('(buffer-name (window-buffer (selected-window)))')
        value = s.emacsclient('-e', '(+ 20 22)').stdout.strip()
        s.emacsclient('-n', '+3:4', str(a))
        position = s.eval(f'(with-current-buffer (get-file-buffer {lisp(a)}) (list (line-number-at-pos) (current-column)))')
        # A text-terminal frame needs a terminal: emacsclient runs under script(1)'s pty.
        tty = subprocess.run(['script', '-qec', f'{args.emacsclient} -s {s.name} -t', '/dev/null'], env={**s.env, 'TERM': 'xterm'},
                             cwd=s.root / 'work', stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=60)
        s.keyboard_ready()
        s.keys('-M', 'ctrl', '-k', 'x', '-k', 'c', '-m', 'ctrl')
        until(lambda: s.eval("(romacs--native-window 'query)") == 'nil', 10, 'window hidden by C-x C-c')
        alive = s.eval('(emacs-pid)')
        s.emacsclient('-c', '-n')
        by_emacsclient = s.eval("(romacs--native-window 'query)")
        s.eval("(romacs--native-window 'hide)")
        paints_before = s.log().count('"diagnostic":"paint')
        again, _ = s.client('-n')
        reshown = s.eval("(romacs--native-window 'query)")
        until(lambda: s.log().count('"diagnostic":"paint') > paints_before, 10, 'a paint after the window shows again')
        s.client('--stop=none')
        until(lambda: not (s.runtime / 'romacs/lc/server.sock').exists(), 15, 'server socket gone')
        until(lambda: not Path(f'/proc/{s.pid}').exists(), 15, 'server process gone')
        result = {'world_seconds': s.world_seconds, 'standalone_ready_s': standalone, 'cold_client_s': round(cold, 3), 'pairs': pairs, 'warm_client_s': round(warm, 3),
                  'shown': shown, 'current': current, 'emacsclient_eval': value, 'position': position,
                  'tty_refused': 'text-terminal' in tty.stdout + tty.stderr, 'tty_reply': (tty.stdout + tty.stderr)[-300:],
                  'kept_after_close': alive == str(s.pid), 'shown_by_emacsclient_c': by_emacsclient, 'reshow_client_s': round(again, 3), 'reshown': reshown}
        result['ok'] = (shown == 't' and current == '"b.txt"' and value == '42' and position == '(3 3)'
                        and result['tty_refused'] and result['kept_after_close'] and by_emacsclient == 't' and reshown == 't' and warm < .3)
        # Reported, not judged here: under a loaded host single launches swing
        # by a second; the gate reads the paired medians.
        result['launch_within_0_2s'] = cold <= standalone + .2
        return result
    finally:
        s.close()


def waiters(args, base):
    s = Server(args, base, 'wt')
    try:
        s.client('-n')
        s.pid = int(s.eval('(emacs-pid)'))
        files = [s.file(f'w{i}.txt') for i in range(3)]
        client = s.waiting(*map(str, files))
        until(lambda: s.eval(f'(and (romacs-server-native-waiting-p (get-file-buffer {lisp(files[2])})) t)') == 't',
              20, 'three files waited on')
        early = []
        for i in (1, 0, 2):
            early.append(client.poll())
            s.eval(f'(progn (switch-to-buffer (get-file-buffer {lisp(files[i])})) (server-edit))')
        client.wait(15)
        ordered = early == [None, None, None] and client.returncode == 0
        gone = s.waiting(str(s.file('gone.txt')))
        until(lambda: s.eval('(hash-table-count romacs-server--peers)') == '1', 20, 'disconnecting client waits')
        gone.kill()
        gone.wait()
        until(lambda: s.eval('(list (hash-table-count romacs-server--peers) (hash-table-count romacs-server--waiters))')
              == '(0 0)', 10, 'disconnected client forgotten')
        mixed = s.file('mixed.txt')
        native = s.waiting(str(mixed))
        gnu = subprocess.Popen([args.emacsclient, '-s', s.name, str(mixed)], env=s.env, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE)
        until(lambda: s.eval(f'(with-current-buffer (get-file-buffer {lisp(mixed)}) '
                             '(and server-buffer-clients (romacs-server-native-waiting-p) t))') == 't',
              20, 'mixed waiters')
        s.eval(f'(progn (switch-to-buffer (get-file-buffer {lisp(mixed)})) (server-edit))')
        native.wait(15)
        gnu.wait(15)
        killed = s.file('killed.txt')
        victim = s.waiting(str(killed))
        until(lambda: s.eval(f'(and (romacs-server-native-waiting-p (get-file-buffer {lisp(killed)})) t)') == 't',
              20, 'kill waiter')
        s.eval(f'(kill-buffer (get-file-buffer {lisp(killed)}))')
        victim.wait(15)
        saved = s.file('saved.txt')
        asker = s.waiting(str(saved))
        until(lambda: s.eval(f'(and (romacs-server-native-waiting-p (get-file-buffer {lisp(saved)})) t)') == 't',
              20, 'save waiter')
        s.eval(f'(progn (switch-to-buffer (get-file-buffer {lisp(saved)})) (goto-char (point-max)) (insert "added\\n"))')
        s.keyboard_ready()
        s.keys('-M', 'ctrl', '-k', 'x', '-m', 'ctrl', '#')
        until(lambda: 'Save file' in s.eval('(or (minibuffer-prompt) (current-message) "")') or None, 10, 'save question')
        s.keys('y')
        asker.wait(15)
        result = {'ordered': ordered, 'mixed': (native.returncode, gnu.returncode), 'killed': victim.returncode,
                  'saved_text': saved.read_text().endswith('added\n'), 'saved_client': asker.returncode}
        result['ok'] = (ordered and result['mixed'] == (0, 0) and victim.returncode == 0
                        and result['saved_text'] and asker.returncode == 0)
        s.client('--stop=none')
        return result
    finally:
        s.close()


def unit(args, base):
    bus = os.environ.get('DBUS_SESSION_BUS_ADDRESS')
    if not bus:
        return {'ok': None, 'skipped': 'no user bus'}
    name = f'u{os.getpid()}'
    s = Server(args, base, name, {'DBUS_SESSION_BUS_ADDRESS': bus, 'ROMACS_SERVER_SLICE': 'romacs-build.slice'})
    # systemd-run reaches the user manager through the real runtime
    # directory; the server's sockets go there too, under this test's name.
    s.env['WAYLAND_DISPLAY'] = str(s.runtime / s.env['WAYLAND_DISPLAY'])
    s.env['XDG_RUNTIME_DIR'] = os.environ.get('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}')
    unit_name = s.unit = f'romacs-server-{name}'
    state = lambda: subprocess.run(['systemctl', '--user', 'is-active', unit_name], capture_output=True,
                                   text=True).stdout.strip()
    try:
        s.client('-n')
        s.pid = int(s.eval('(emacs-pid)'))
        started = state()
        s.keyboard_ready()
        s.keys('-M', 'ctrl', '-k', 'x', '-k', 'c', '-m', 'ctrl')
        until(lambda: s.eval("(romacs--native-window 'query)") == 'nil', 10, 'window hidden')
        kept = state()
        s.client('--stop=none')
        until(lambda: state() != 'active', 15, 'unit stopped')
        return {'started': started, 'kept_after_close': kept, 'after_stop': state(),
                'fallback_log': s.log()[:200],
                'ok': started == 'active' and kept == 'active' and state() != 'active'}
    finally:
        subprocess.run(['systemctl', '--user', 'stop', unit_name], capture_output=True)
        s.close()


PROBE = '''(let ((times nil))
  (dolist (clients '(1 10 100))
    (dolist (files '(1 10 100))
      (let* ((buffers (cl-loop for i below files collect (get-buffer-create (format " probe-%d-%d-%d" clients files i))))
             (peers (cl-loop for c below clients collect (list 'probe-peer c)))
             (all (cl-loop for peer in peers collect (romacs-server--wait (list :probe t) peer buffers))))
        (cl-letf (((symbol-function 'romacs-mcp-reply) #'ignore))
          (let ((start (float-time)))
            (romacs-server--release (car buffers))
            (push (list clients files 'finish-one (- (float-time) start)) times))
          (let ((start (float-time)))
            (romacs-server--peer-closed (car peers))
            (push (list clients files 'retire-one (- (float-time) start)) times))
          (dolist (peer (cdr peers)) (romacs-server--peer-closed peer)))
        (mapc #'kill-buffer buffers)
        (ignore all))))
  (cl-letf (((symbol-function 'romacs-mcp-reply) #'ignore))
    ;; C-x #'s own path (server-done) through every file of one client, in order.
    (dolist (files '(1 10 100))
      (let ((buffers (cl-loop for i below files collect (get-buffer-create (format " probe-done-%d-%d" files i)))))
        (romacs-server--wait (list :probe t) 'probe-done buffers)
        (let ((start (float-time)))
          (dolist (buffer buffers) (with-current-buffer buffer (server-done)))
          (push (list 1 files 'done-each (/ (- (float-time) start) files)) times))
        (mapc #'kill-buffer buffers)))
    ;; Many requests held on one connection: finishing one.
    (dolist (clients '(1 10 100))
      (let ((buffers (cl-loop for i below clients collect (get-buffer-create (format " probe-peer-%d-%d" clients i)))))
        (dolist (buffer buffers) (romacs-server--wait (list :probe t) 'probe-one-peer (list buffer)))
        (let ((start (float-time)))
          (romacs-server--release (car (last buffers)))
          (push (list clients 1 'same-peer-finish (- (float-time) start)) times))
        (romacs-server--peer-closed 'probe-one-peer)
        (mapc #'kill-buffer buffers)))
    ;; Out of order: files 2..N finished first, then the first one with C-x #.
    (dolist (files '(1 10 100 1000))
      (let ((buffers (cl-loop for i below files collect (get-buffer-create (format " probe-rev-%d-%d" files i)))))
        (romacs-server--wait (list :probe t) 'probe-rev buffers)
        (mapc #'romacs-server--release (cdr buffers))
        (let ((start (float-time)))
          (with-current-buffer (car buffers) (server-done))
          (push (list 1 files 'last-done (- (float-time) start)) times))
        (mapc #'kill-buffer buffers)))
    ;; Disconnected clients leave nothing behind.
    (dotimes (i 100)
      (let ((buffer (get-buffer-create (format " probe-gone-%d" i))) (peer (list 'probe-gone i)))
        (romacs-server--wait (list :probe t) peer (list buffer))
        (romacs-server--peer-closed peer)
        (kill-buffer buffer)))
    (push (list 100 1 'nodes-left (hash-table-count romacs-server--nodes)) times))
  (prin1-to-string (nreverse times)))'''


def probe(args, base):
    s = Server(args, base, 'pr')
    try:
        s.client('-n')
        s.pid = int(s.eval('(emacs-pid)'))
        rows = parity_live.read_sexp(parity_live.read_sexp(s.eval(PROBE)))
        s.client('--stop=none')
        table = [{'clients': c, 'files': f, 'op': str(op), 'us': round(t * 1e6, 1)} for c, f, op, t in rows]
        # Finishing one buffer costs its waiters (clients), never other files;
        # retiring one client costs its files, never other clients.
        def cost(op, clients, files):
            return next(r['us'] for r in table if (r['op'], r['clients'], r['files']) == (op, clients, files))
        flat = all(cost('finish-one', c, 100) < 20 * cost('finish-one', c, 1) + 200 for c in (1, 10, 100)) and \
            all(cost('retire-one', 100, f) < 20 * cost('retire-one', 1, f) + 200 for f in (1, 10, 100)) and \
            cost('done-each', 1, 100) < 20 * cost('done-each', 1, 1) + 200 and \
            cost('same-peer-finish', 100, 1) < 20 * cost('same-peer-finish', 1, 1) + 200 and \
            cost('last-done', 1, 1000) < 20 * cost('last-done', 1, 1) + 200 and \
            next(r['us'] for r in table if r['op'] == 'nodes-left') == 0
        return {'table': table, 'ok': flat}
    finally:
        s.close()


def stop(args, base):
    """--stop=all saves every modified file whatever the save predicate
    says; --stop=ask shows the window to ask in."""
    s = Server(args, base, 'st')
    try:
        f = s.file('kept.txt')
        s.client('-n', str(f))
        s.pid = int(s.eval('(emacs-pid)'))
        s.eval(f'(progn (setq save-some-buffers-default-predicate (lambda () nil)) '
               f'(with-current-buffer (get-file-buffer {lisp(f)}) (goto-char (point-max)) (insert "edited\\n")) '
               "(romacs--native-window 'hide))")
        s.client('--stop=all')
        until(lambda: not Path(f'/proc/{s.pid}').exists(), 15, 'server stopped after saving')
        saved = f.read_text().endswith('edited\n')
        g = s.file('asked.txt')
        s.client('-n', str(g))
        s.pid = int(s.eval('(emacs-pid)'))
        s.eval(f"(progn (with-current-buffer (get-file-buffer {lisp(g)}) (insert \"x\")) (romacs--native-window 'hide))")
        s.client('--stop')
        until(lambda: s.eval("(romacs--native-window 'query)") == 't', 10, 'window shown to ask')
        asking = Path(f'/proc/{s.pid}').exists()
        s.client('--stop=none')
        until(lambda: not Path(f'/proc/{s.pid}').exists(), 15, 'server stopped')
        return {'all_saved_despite_predicate': saved, 'ask_showed_window_and_waited': asking,
                'ok': saved and asking}
    finally:
        s.close()


def notifier(root):
    """A stand-in notify-send recording its arguments, first on PATH."""
    bin_dir, log = root / 'notify-bin', root / 'notifications.jsonl'
    bin_dir.mkdir()
    script = bin_dir / 'notify-send'
    script.write_text(f'#!/bin/sh\nprintf "%s\\0" "$@" >> {log}\nprintf "\\n" >> {log}\n')
    script.chmod(0o755)
    return bin_dir, log


def notified(log):
    if not log.exists():
        return []
    return [line.strip('\0').split('\0') for line in log.read_text().splitlines() if line]


def crash(args, base):
    bus = os.environ.get('DBUS_SESSION_BUS_ADDRESS')
    if not bus:
        return {'ok': None, 'skipped': 'no user bus'}
    name = f'c{os.getpid()}'
    s = Server(args, base, name, {'DBUS_SESSION_BUS_ADDRESS': bus, 'ROMACS_SERVER_SLICE': 'romacs-build.slice'})
    bin_dir, log = notifier(s.root)
    s.env['PATH'] = f'{bin_dir}:{s.env["PATH"]}'
    s.env['WAYLAND_DISPLAY'] = str(s.runtime / s.env['WAYLAND_DISPLAY'])
    s.env['XDG_RUNTIME_DIR'] = os.environ.get('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}')
    unit_name = s.unit = f'romacs-server-{name}'
    state = lambda: subprocess.run(['systemctl', '--user', 'is-active', unit_name], capture_output=True,
                                   text=True).stdout.strip()
    try:
        s.client('-n')
        s.pid = int(s.eval('(emacs-pid)'))
        os.kill(s.pid, 9)
        until(lambda: notified(log), 20, 'crash notification')
        crashed = notified(log)
        # A clean stop and a unit stop (as at logout) are no news.
        s.client('-n')
        s.client('--stop=none')
        until(lambda: state() != 'active', 15, 'stopped')
        s.client('-n')
        subprocess.run(['systemctl', '--user', 'stop', unit_name], capture_output=True, timeout=30)
        until(lambda: state() != 'active', 15, 'unit stopped')
        time.sleep(1)
        after = notified(log)
        expected = ['-u', 'critical', '-a', 'Romacs', 'Romacs exited (signal KILL)',
                    f'Log: journalctl --user -u {unit_name}']
        return {'crash': crashed, 'clean_ends_noticed': len(after) - len(crashed),
                'ok': crashed == [expected] and after == crashed}
    finally:
        subprocess.run(['systemctl', '--user', 'stop', unit_name], capture_output=True)
        s.close()


def build(args, base):
    builds = base / 'builds'
    for label, commit in (('a', 'aaaaaaaa11111111'), ('b', 'bbbbbbbb22222222'), ('c', 'cccccccc33333333')):
        (builds / label).mkdir(parents=True)
        (builds / label / 'build.json').write_text(json.dumps({'commit': commit}))
    link = base / 'current'
    link.symlink_to(builds / 'a')

    def install(label):
        fresh = base / 'current.new'
        fresh.symlink_to(builds / label)
        fresh.replace(link)

    s = Server(args, base, 'bw', {'ROMACS_BUILD_WATCH': str(link)})
    told = lambda: lisp_read(s.eval('(with-current-buffer "*Messages*" (buffer-string))'))
    try:
        s.client('-n')
        s.pid = int(s.eval('(emacs-pid)'))
        quiet_first = 'installed' not in told()
        install('b')
        until(lambda: 'Romacs bbbbbbbb installed; quit and relaunch to switch' in told(), 20, 'notice')
        time.sleep(4.5)  # more checks: said once
        once = told().count('installed') == 1
        toast = lisp_read(s.eval('(if (boundp (quote romacs-layer--toasts)) '
                                 '(mapconcat (lambda (toast) (nth 1 toast)) romacs-layer--toasts "|") "")'))
        # Hidden: nothing is said until the window shows again.
        s.eval("(romacs--native-window 'hide)")
        install('c')
        time.sleep(4.5)  # two checks while hidden
        hidden_quiet = 'cccccccc' not in told()
        s.client('-n')
        until(lambda: 'Romacs cccccccc installed' in told(), 20, 'notice once shown')
        result = {'quiet_first': quiet_first, 'once': once, 'toast': toast, 'hidden_quiet': hidden_quiet,
                  'pid_same': s.eval('(emacs-pid)') == str(s.pid)}
        result['ok'] = quiet_first and once and hidden_quiet and result['pid_same']
        return result
    finally:
        s.close()


def lisp_read(text):
    return parity_live.read_sexp(text) if text.startswith('"') else text


WORKFLOWS = {'stop': stop, 'lifecycle': lifecycle, 'waiters': waiters, 'unit': unit, 'crash': crash, 'build': build,
             'probe': probe}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--emacsclient', default=str(Path(os.environ.get('ROMACS_GNU_ORACLE', 'emacs')).parent / 'emacsclient'))
    parser.add_argument('--workflow', action='append', choices=sorted(WORKFLOWS))
    args = parser.parse_args()
    out = Path(args.output)
    out.mkdir(parents=True, exist_ok=False)
    results = {}
    for name in args.workflow or ['lifecycle', 'waiters', 'stop', 'build', 'probe']:
        try:
            results[name] = WORKFLOWS[name](args, out)
        except Exception as error:  # a workflow's failure is its result
            results[name] = {'ok': False, 'error': repr(error)}
        print(name, json.dumps(results[name]), flush=True)
    (out / 'summary.json').write_text(json.dumps(results, indent=2))
    return 0 if all(r.get('ok') is not False for r in results.values()) else 1


if __name__ == '__main__':
    sys.exit(main())
