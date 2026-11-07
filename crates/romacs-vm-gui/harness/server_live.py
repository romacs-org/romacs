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
  landing    the landing comes back from real keys: q kills *Romacs*, M-x
             `romacs-landing-show' builds it again with its keys and motion,
             and shows the live one without rebuilding it; a client with no
             files lands on an empty session, by `romacs' and `emacsclient
             -c', and not with `initial-buffer-choice' or
             `inhibit-startup-screen' set, nor beside work.
  first_frame  no frame of a launch shows *scratch* before the landing, by the
             launcher's client path and a plain launch: every frame the host
             submits, from its scene dumps; a window shown again to a new
             client, by `romacs' and `emacsclient -c', judged as well from
             screenshots, which see what the renderer presents.  A launch that
             opts out (`inhibit-startup-screen') shows *scratch*; one with a
             file never shows the landing.
  daemon     pinned GNU `--fg-daemon=dm' and a Romacs server dm, in turn in
             the same tree: `noninteractive', `daemonp' and
             `daemon-initialized' before and after `emacsclient -c' are
             GNU's, and `ding' with no frame, with one and in a worker
             isolate writes no BEL to the server's output; with its window
             shown, `server-stop-automatically' `empty' keeps the server.
  title      `frame-title-format' reaches the window, again when the buffer's
             name and the format change.
  mouse      `mouse-pixel-position' follows the real pointer over the window,
             and `mouse-position' finds characters from it as pinned GNU's.

usage: server_live.py --romacs BIN --output DIR [--workflow NAME]... [--tools DIR]
"""
import argparse
import json
import os
import re
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time

import parity_live
from parity_live import prepare_world, private_tree, wait

INIT = ''';; -*- lexical-binding: t -*-
(setq inhibit-startup-screen t initial-scratch-message nil)
'''


class Server:
    """A private tree with a compositor, in which clients start server NAME."""

    def __init__(self, args, base, name, env_extra=None, init=INIT):
        self.args, self.name = args, name
        self.root, self.home, self.env = private_tree('romacs', base, args.tools, prefix=f'server-{name}-')
        # Sockets need short paths (108 bytes): a runtime directory of their
        # own on the host's, while the tree (worlds, journals) stays on disk.
        host_runtime = Path(os.environ.get('XDG_RUNTIME_DIR') or f'/run/user/{os.getuid()}')
        self.runtime = Path(tempfile.mkdtemp(prefix='rs-', dir=host_runtime))
        self.env['XDG_RUNTIME_DIR'] = str(self.runtime)
        (self.home / '.config/romacs').mkdir()
        (self.home / '.config/romacs/init.el').write_text(init)
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
        display = wait(lambda: next((p for p in self.runtime.glob('wayland-*') if p.is_socket()), None),
                      15, 'compositor', self.compositor)
        self.env['WAYLAND_DISPLAY'] = display.name
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


def landing(args, base):
    # The landing itself: the screen left on, as a new user's is.
    s = Server(args, base, 'ld', init=';; -*- lexical-binding: t -*-\n(setq initial-scratch-message nil)\n')
    current = lambda: s.eval('(buffer-name (window-buffer (selected-window)))')

    def typed_show():
        s.keys('-M', 'alt', '-k', 'x', '-m', 'alt')
        s.keys('romacs-landing-show')
        s.keys('-k', 'Return')
        until(lambda: current() == '"*Romacs*"', 15, 'the landing shows')

    def quit_landing():
        s.keys('-k', 'q')
        until(lambda: s.eval('(get-buffer "*Romacs*")') == 'nil', 15, 'q kills the landing')

    try:
        s.client('-n')
        started = current()
        buffers = s.eval("(mapcar #'buffer-name (buffer-list))")
        s.keyboard_ready()
        quit_landing()
        typed_show()
        state = s.eval('''(with-current-buffer "*Romacs*"
                            (list buffer-read-only cursor-type (key-binding " ") (key-binding "q")
                                  (and (romacs-animate--get 'romacs-landing) t)
                                  (equal default-directory command-line-default-directory)))''')
        # Shown again, the live buffer is reused.
        s.eval('(progn (setq ld-buffer (get-buffer "*Romacs*")) (switch-to-buffer "*scratch*"))')
        typed_show()
        reused = s.eval('(eq ld-buffer (get-buffer "*Romacs*"))')
        # A client with no files, on an empty session.
        quit_landing()
        s.client('-n')
        native = current()
        quit_landing()
        s.emacsclient('-c', '-n')
        client = current()
        # Not where the user chose otherwise, nor beside work.
        quit_landing()
        def clients():
            """Where each kind of client leaves the window, judged alone."""
            s.client('-n')
            native = current()
            s.emacsclient('-c', '-n')
            return [native, current()]

        s.eval('(setq initial-buffer-choice t)')
        chosen = clients()
        s.eval('(setq initial-buffer-choice nil inhibit-startup-screen t)')
        inhibited = clients()
        s.eval('(setq inhibit-startup-screen nil)')
        # A function that clears the choice still chose.
        s.eval('(setq initial-buffer-choice (lambda () (setq initial-buffer-choice nil) (get-scratch-buffer-create)))')
        s.emacsclient('-c', '-n')
        cleared = current()
        # A client arriving over a prompt or a search leaves it be.
        s.keys('-M', 'alt', '-k', 'x', '-m', 'alt')
        until(lambda: s.eval('(and (active-minibuffer-window) t)') == 't', 15, 'a prompt opens')
        s.client('-n')
        prompt = s.eval('(list (and (active-minibuffer-window) t) (minibufferp (window-buffer (selected-window))))')
        s.keys('-M', 'ctrl', '-k', 'g', '-m', 'ctrl')
        until(lambda: s.eval('(active-minibuffer-window)') == 'nil', 15, 'the prompt closes')
        s.keys('-M', 'ctrl', '-k', 's', '-m', 'ctrl')
        until(lambda: s.eval('isearch-mode') != 'nil', 15, 'a search starts')
        s.client('-n')
        search = [s.eval('(and isearch-mode t)'), current()]
        s.keys('-M', 'ctrl', '-k', 'g', '-m', 'ctrl')
        until(lambda: s.eval('isearch-mode') == 'nil', 15, 'the search ends')
        a = s.file('a.txt')
        s.client('-n', str(a))
        s.client('-n')
        beside_native = current()
        s.emacsclient('-c', '-n')
        beside_client = current()
        s.client('--stop=none')
        result = {'started': started, 'buffers': buffers, 'state': state, 'reused': reused, 'native': native,
                  'emacsclient': client, 'chosen': chosen, 'inhibited': inhibited, 'cleared': cleared, 'prompt': prompt,
                  'search': search, 'beside_native': beside_native, 'beside_client': beside_client}
        result['ok'] = (started == '"*Romacs*"' and state == '(t nil romacs-landing-pause exit-splash-screen t t)'
                        and reused == 't' and native == client == '"*Romacs*"' and chosen == inhibited == ['"*scratch*"'] * 2
                        and cleared == '"*scratch*"' and prompt == '(t t)' and search == ['t', '"*scratch*"']
                        and beside_native == '"a.txt"' and beside_client == '"*scratch*"')
        return result
    finally:
        s.close()


def shown_buffers(log, start=0):
    """From LOG, the scene dumps of `--trace-scene' from character START on:
    for each frame, which of the two buffers it shows."""
    frames = []
    for line in log[start:].split('\n'):
        if '"submitted-scene"' not in line:
            continue
        try:
            scene = json.loads(line)
        except json.JSONDecodeError:
            continue
        rows = {}
        for glyph in scene['glyphs']:
            rows.setdefault(glyph['y'], []).append((glyph['x'], glyph.get('char') or ''))
        text = '\n'.join(''.join(c for _, c in sorted(row)) for _, row in sorted(rows.items()))
        frames.append('*Romacs*' if '*Romacs*' in text else '*scratch*' if '*scratch*' in text else 'other')
    return frames


def first_frame(args, base):
    plain_init = ';; -*- lexical-binding: t -*-\n'
    s = Server(args, base, 'ff', init=plain_init)
    current = lambda: s.eval('(buffer-name (window-buffer (selected-window)))')
    show = ('--trace-scene', '-n')

    def bar(name):
        """A screenshot's status bar, where a buffer's name is, as a file."""
        path = s.root / f'{name}.png'
        subprocess.run([f'{args.tools}/grim', str(path)], env=s.env, check=True, timeout=15)
        subprocess.run(['magick', str(path), '-gravity', 'South', '-crop', '400x40+0+0', '+repage', str(path)], check=True, timeout=15)
        return path

    def differs(a, b):
        done = subprocess.run(['magick', 'compare', '-metric', 'AE', str(a), str(b), 'null:'], capture_output=True, text=True, timeout=15)
        return float(done.stderr.split()[0])

    def burst(work, reference, name):
        """Screenshots while WORK runs, and how many show each of REFERENCE,
        the bars over *scratch* and the landing: a shot far from both, before
        the window shows, counts as neither."""
        done = threading.Event()
        shots = []

        def shoot():
            while not done.is_set() and len(shots) < 40:
                try:
                    shots.append(bar(f'{name}-{len(shots)}'))
                except subprocess.CalledProcessError:
                    pass  # no output yet
        thread = threading.Thread(target=shoot)
        thread.start()
        work()
        time.sleep(1.5)
        done.set()
        thread.join()
        near = differs(*reference) * 2
        if not near:
            raise RuntimeError('the two bars look alike')
        apart = [(differs(shot, reference[0]), differs(shot, reference[1])) for shot in shots]
        return len(shots), sum(s < min(near, l) for s, l in apart), sum(l < min(near, s) for s, l in apart)

    try:
        # The launcher's path: a client starts the server.
        s.client(*show)
        until(lambda: shown_buffers(s.log()), 20, 'a frame')
        launcher = shown_buffers(s.log())
        s.keyboard_ready()
        landing_bar = bar('ref-landing')
        s.keys('-k', 'q')
        until(lambda: current() == '"*scratch*"', 15, 'q leaves the landing')
        time.sleep(1)
        scratch_bar = bar('ref-scratch')
        # A window shown again to a new client, of either kind.
        again = {}
        for kind, work in (('native', lambda: s.client('-n')), ('emacsclient', lambda: s.emacsclient('-c', '-n'))):
            s.eval("(progn (when (get-buffer \"*Romacs*\") (kill-buffer \"*Romacs*\")) (romacs--native-window 'hide))")
            until(lambda: s.eval("(romacs--native-window 'query)") == 'nil', 10, 'the window hides')
            time.sleep(.5)
            start = len(s.log())
            shots, scratch, landing_shots = burst(work, (scratch_bar, landing_bar), kind)
            again[kind] = {'frames': shown_buffers(s.log(), start), 'shots': shots, 'scratch_shots': scratch,
                           'landing_shots': landing_shots, 'current': current()}
        s.client('--stop=none')
        until(lambda: not (s.runtime / 'romacs/ff/server.sock').exists(), 15, 'the server stops')
        # A file: the landing never shows.
        a = s.file('a.txt')
        s.client('--trace-scene', '-n', str(a))
        until(lambda: shown_buffers(s.log()), 20, 'a frame')
        time.sleep(1)
        with_file = shown_buffers(s.log())
        s.client('--stop=none')
        until(lambda: not (s.runtime / 'romacs/ff/server.sock').exists(), 15, 'the server stops')
        # A plain launch, no server.
        log = s.root / 'plain.log'
        with log.open('w') as out:
            plain = subprocess.Popen([args.romacs, '--loadup', '--trace-scene'], env=s.env, cwd=s.root / 'work',
                                     stdin=subprocess.DEVNULL, stdout=out, stderr=out)
        try:
            until(lambda: shown_buffers(log.read_text(errors='replace')), 30, 'a plain launch paints')
            time.sleep(1)
        finally:
            plain.terminate()
            plain.wait(10)
        direct = shown_buffers(log.read_text(errors='replace'))
    finally:
        s.close()
    # A launch that opts out, as the harnesses do, lands on *scratch*.
    s = Server(args, base, 'fo')
    try:
        s.client(*show)
        until(lambda: shown_buffers(s.log()), 20, 'a frame')
        opted_out = shown_buffers(s.log())
        s.client('--stop=none')
    finally:
        s.close()
    only = lambda frames, buffer: bool(frames) and set(frames) == {buffer}
    landing_only = lambda frames: only(frames, '*Romacs*')
    result = {'launcher': launcher, 'again': again, 'plain': direct, 'with_file': with_file, 'opted_out': opted_out}
    result['ok'] = (landing_only(launcher) and landing_only(direct) and '*Romacs*' not in with_file
                    and all(landing_only(a['frames']) and a['scratch_shots'] == 0 and a['landing_shots'] > 0 for a in again.values())
                    and only(opted_out, '*scratch*'))
    return result


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


DAEMON_PROBE = '(list noninteractive (daemonp) (condition-case e (daemon-initialized) (error (cadr e))))'
# What `make-frame' reaches (`pgtk-dnd-init-frame'), and the primitive's
# checks, on a window frame.
DND_PROBE = '''(let ((f (car (filtered-frame-list #'display-graphic-p))))
  (append
   (mapcar (lambda (targets) (condition-case e (pgtk-register-dnd-targets f targets) (error e)))
           (list '("text/plain") nil 'x '("a" . "b") '("a" 3)))
   (let ((made (make-frame `((parent-frame . ,f) (visibility . nil)))))
     (prog1 (list (frame-live-p made)) (delete-frame made)))))'''


def daemon(args, base):
    """GNU's daemon and Romacs's server, asked the same before and after a
    client's frame; `ding' rings in both states.  The frame counts are
    recorded, not compared: Romacs's frames are its own (DIVERGENCES.md)."""
    s = Server(args, base, 'dm')
    try:
        ask = lambda form: s.emacsclient('-e', form).stdout.strip()

        def states(shown):
            before = (ask(DAEMON_PROBE), ask('(length (frame-list))'))
            ask('(ding)')
            s.emacsclient('-c', '-n')
            until(lambda: ask(shown) == 't', 30, 'the client frame shown')
            after = (ask(DAEMON_PROBE), ask('(length (frame-list))'))
            ask('(ding)')
            return before, after, ask(DND_PROBE)
        gnu_out = s.root / 'gnu-daemon.out'
        with gnu_out.open('wb') as out:
            gnu = subprocess.Popen([args.gnu, '-Q', '--fg-daemon=dm'], env=s.env, cwd=s.root / 'work',
                                   stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT)
            try:
                until(lambda: s.emacsclient('-e', 't', check=False).returncode == 0, 60, 'GNU daemon listening')
                gnu_states = states('(and (cdr (frame-list)) t)')
                s.emacsclient('-e', '(kill-emacs)', check=False)
                gnu.wait(timeout=30)
            finally:
                gnu.kill()
        s.client('-e', 't')
        s.pid = int(s.eval('(emacs-pid)'))
        romacs_states = states("(romacs--native-window 'query)")
        # A worker isolate is batch Lisp, but its output is no one's: the
        # line ends so a written BEL would reach the server's log.
        worker = ask("(isolate-await (isolate-spawn '(lambda () (ding) (terpri) noninteractive) :fresh t :allow-source-boot t) 120)")
        gnu_bells, romacs_bells = gnu_out.read_bytes().count(b'\x07'), s.log().count('\x07')
        # As a daemon, server.el's `server-stop-automatically' applies: `empty'
        # stops a session left with no client frame; one showing its window
        # is past the timer's first run (10 s) and still serving.
        ask("(customize-set-variable 'server-stop-automatically 'empty)")
        time.sleep(13)
        survived = s.emacsclient('-e', '(list (emacs-pid) (timerp server-stop-automatically--timer))',
                                 check=False).stdout.strip() == f'({s.pid} t)'
        result = {'gnu': gnu_states, 'romacs': romacs_states, 'worker_noninteractive': worker,
                  'gnu_bells': gnu_bells, 'romacs_bells': romacs_bells, 'stop_automatically_survived': survived}
        result['ok'] = ([b[0] for b in gnu_states[:2]] == [b[0] for b in romacs_states[:2]] and gnu_states[2] == romacs_states[2]
                        and worker == 't' and gnu_bells == 0 and romacs_bells == 0 and survived)
        return result
    finally:
        s.close()

def title(args, base):
    """`frame-title-format' reaches the window: what a Romacs server tells
    its host after a client's frame shows and the buffer changes, against
    GNU's `format-mode-line' of the same format in the same buffer names."""
    s = Server(args, base, 'tt', {'ROMACS_DIAGNOSTICS': '1'})
    try:
        a = s.file('a.txt')
        s.client('-n', str(a))
        s.pid = int(s.eval('(emacs-pid)'))
        titles = lambda: [json.loads(line)['window_title'] for line in s.log().splitlines() if line.startswith('{"window_title"')]
        host = s.eval('(system-name)').strip('"')
        wanted = f'a.txt - Romacs at {host}'
        until(lambda: wanted in titles(), 30, f'the title {wanted}')
        s.eval('(with-current-buffer "a.txt" (rename-buffer "renamed.txt"))')
        renamed = f'renamed.txt - Romacs at {host}'
        s.eval('(force-mode-line-update t)')
        s.keys('-M', 'ctrl', '-k', 'g', '-m', 'ctrl')
        until(lambda: renamed in titles(), 30, f'the title {renamed}')
        # With nothing to paint: a client's form, then a timer's.
        s.eval("(setq frame-title-format \"custom %b\")")
        until(lambda: 'custom renamed.txt' in titles(), 30, 'the custom title')
        s.eval("(run-at-time 0.1 nil (lambda () (setq frame-title-format \"timer %b\")))")
        until(lambda: 'timer renamed.txt' in titles(), 30, 'the timer title')
        # A construct the key does not cover: the size follows each edit.
        s.eval('(with-current-buffer "renamed.txt" (setq frame-title-format "%b (%i)") (set-buffer-modified-p t) (erase-buffer) (insert "a"))')
        until(lambda: 'renamed.txt (1)' in titles(), 30, 'the size in the title')
        s.eval('(with-current-buffer "renamed.txt" (insert "b"))')
        until(lambda: 'renamed.txt (2)' in titles(), 30, 'the next size in the title')
        # %* reads `buffer-read-only' and the modified state.
        s.eval('(with-current-buffer "renamed.txt" (setq frame-title-format "%b %*") (set-buffer-modified-p nil))')
        until(lambda: 'renamed.txt -' in titles(), 30, 'the unmodified mark')
        s.eval('(with-current-buffer "renamed.txt" (setq buffer-read-only t))')
        until(lambda: 'renamed.txt %' in titles(), 30, 'the read-only mark')
        return {'titles': titles(), 'ok': True}
    finally:
        s.close()


class VirtualPointer:
    """A pointer on the private compositor, over zwlr_virtual_pointer_v1:
    the Wayland wire protocol by hand, as the tools have no pointer and a
    seat without one sends a client no pointer events."""

    def __init__(self, env):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.connect(str(Path(env['XDG_RUNTIME_DIR']) / env['WAYLAND_DISPLAY']))
        self.send(1, 1, struct.pack('<I', 2))  # wl_display.get_registry -> 2
        self.send(1, 0, struct.pack('<I', 3))  # wl_display.sync -> 3
        name = None
        for sender, opcode, body in self.events(3):
            if sender == 2 and opcode == 0:  # wl_registry.global
                length = struct.unpack_from('<I', body, 4)[0]
                if body[8:8 + length - 1] == b'zwlr_virtual_pointer_manager_v1':
                    name = struct.unpack_from('<I', body)[0]
        if name is None:
            raise RuntimeError('the compositor has no virtual pointer')
        interface = b'zwlr_virtual_pointer_manager_v1\0'
        padded = interface + bytes(-len(interface) % 4)
        self.send(2, 0, struct.pack('<I', name) + struct.pack('<I', len(interface)) + padded + struct.pack('<II', 1, 4))
        self.send(4, 0, struct.pack('<II', 0, 5))  # create_virtual_pointer (no seat) -> 5

    def send(self, target, opcode, body=b''):
        self.sock.sendall(struct.pack('<II', target, (8 + len(body)) << 16 | opcode) + body)

    def events(self, until_callback):
        data = b''
        while True:
            data += self.sock.recv(65536)
            while len(data) >= 8 and len(data) >= (struct.unpack_from('<I', data, 4)[0] >> 16):
                sender, word = struct.unpack_from('<II', data)
                size = word >> 16
                yield sender, word & 0xffff, data[8:size]
                data = data[size:]
                if sender == until_callback:
                    return

    def move(self, x, y, extent=(1280, 900)):
        """motion_absolute, then frame."""
        self.send(5, 1, struct.pack('<IIIII', int(time.time() * 1000) & 0xffffffff, x, y, *extent))
        self.send(5, 4)


MOUSE_PROBE = """(with-selected-frame (car (filtered-frame-list #'display-graphic-p))
  (list (cdr (mouse-pixel-position)) (cdr (mouse-position))
        (frame-char-width) (frame-char-height) (frame-internal-border-width)))"""


def mouse(args, base):
    """`mouse-pixel-position' and `mouse-position' read the real pointer,
    moved by a virtual pointer: in pinned GNU's daemon, then a Romacs
    server, each frame's characters found from its pixels the same way."""
    s = Server(args, base, 'ms')
    try:
        def seen(ask):
            pointer = VirtualPointer(s.env)
            answers = []
            for at in ((100, 60), (300, 200)):
                before, last = ask(), []

                def moved():
                    pointer.move(*at)
                    last[:] = [ask()]
                    return last[0] if last[0] != before else None
                answers.append(until(moved, 15, f'the pointer at {at}: {last}'))
            return answers

        def consistent(answer):
            x, y, column, row, width, height, border = map(int, re.findall(r'-?\d+', answer))
            return (column, row) == ((x - border) // width, (y - border) // height)
        ask = lambda: s.emacsclient('-e', MOUSE_PROBE).stdout.strip().replace('\n', ' ')
        gnu = subprocess.Popen([args.gnu, '-Q', '--fg-daemon=ms'], env=s.env, cwd=s.root / 'work',
                               stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            until(lambda: s.emacsclient('-e', 't', check=False).returncode == 0, 60, 'GNU daemon listening')
            s.emacsclient('-c', '-n')
            until(lambda: s.emacsclient('-e', '(and (filtered-frame-list (function display-graphic-p)) t)').stdout.strip() == 't',
                  30, 'the client frame')
            gnu_seen = seen(ask)
            s.emacsclient('-e', '(kill-emacs)', check=False)
            gnu.wait(timeout=30)
        finally:
            gnu.kill()
        s.client('-n', str(s.file('a.txt')))
        s.pid = int(s.eval('(emacs-pid)'))
        s.keyboard_ready()
        romacs_seen = seen(ask)
        # A hidden window has no pointer over it, nor a shown one till it moves.
        s.eval("(romacs--native-window 'hide)")
        hidden = s.eval('(cdr (mouse-pixel-position))')
        s.eval("(romacs--native-window 'show)")
        shown = s.eval('(cdr (mouse-pixel-position))')
        pixels = [re.match(r'\(\(\d+ \. \d+\)', answer).group() for answer in romacs_seen]
        return {'gnu': gnu_seen, 'romacs': romacs_seen, 'hidden': hidden, 'shown': shown,
                'ok': all(map(consistent, gnu_seen + romacs_seen)) and pixels == ['((100 . 60)', '((300 . 200)'] and hidden == '(nil)'}
    finally:
        s.close()


def lisp_read(text):
    return parity_live.read_sexp(text) if text.startswith('"') else text


WORKFLOWS = {'stop': stop, 'lifecycle': lifecycle, 'waiters': waiters, 'unit': unit, 'crash': crash, 'build': build,
             'probe': probe, 'daemon': daemon, 'title': title, 'mouse': mouse, 'landing': landing, 'first_frame': first_frame}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--emacsclient', default=str(Path(os.environ.get('ROMACS_GNU_ORACLE', 'emacs')).parent / 'emacsclient'))
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE', 'emacs'))
    parser.add_argument('--workflow', action='append', choices=sorted(WORKFLOWS))
    args = parser.parse_args()
    out = Path(args.output)
    out.mkdir(parents=True, exist_ok=False)
    results = {}
    for name in args.workflow or ['lifecycle', 'waiters', 'landing', 'first_frame', 'stop', 'build', 'probe']:
        try:
            results[name] = WORKFLOWS[name](args, out)
        except Exception as error:  # a workflow's failure is its result
            results[name] = {'ok': False, 'error': repr(error)}
        print(name, json.dumps(results[name]), flush=True)
    (out / 'summary.json').write_text(json.dumps(results, indent=2))
    return 0 if all(r.get('ok') is not False for r in results.values()) else 1


if __name__ == '__main__':
    sys.exit(main())
