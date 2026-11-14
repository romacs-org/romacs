#!/usr/bin/env python3
"""A terminal outlives its session: the restart-keeps-the-shell journey.

One isolated session (private labwc, HOME, XDG and TMPDIR; restart_live.py's
own setup) runs a romacs-term terminal whose program is a counting script
on a real pty: one line and one timestamp every 0.1 s.  The session is then
replaced and the terminal must be back in the new one:

  default        M-x romacs-restart;
  --kill         kill -9 of the session and a relaunch (no image: the new
                 session adopts the terminal it finds in the keeper);
  --keeper-exec  the keeper execs itself in place first;
  --kill-helper  kill -9 of the terminal's helper process first: the
                 terminal must end, the keeper's row say how, and the
                 session let the handle go, which hangs up the shell left
                 on the kept pty;
  --refuse       a successor that reads other helper versions: the restart
                 must be refused, naming the terminal, and nothing change;
  --terminals N  N terminals, the extra ones with a full scrollback;
  --cat FILE     no restart: time `cat FILE' in a terminal.
  --deep-runtime the session's runtime directory is deeper than a socket
                 address holds (romacs#122), so the keeper's socket path is
                 too; the compositor's stays short.

A run passes when the script's process is the same and alive, its ticks
show no gap over 0.25 s, *romacs-term* is running in the new session and
shows every line printed while no session had it, and goes on showing new
ones.  report.json holds that as `k2_shell', beside the time line, who
holds the pty, the helper's and the keeper's memory, the session's threads
by name, and how many updates came whole and how many as deltas.

Not here: the session itself as a systemd user unit.  A unit given this
harness's private runtime directory cannot reach the user manager, so
romacs-restart cannot start its successor as a unit; given the real one it
would see the user's own keeper and sockets.  That journey needs a user
manager of the harness's own.

Beside the terminal the session also carries a pipe subprocess and a TCP
connection, stand-ins for what later slices keep; they end at the restart
today and are only recorded.

usage: term_keep_live.py --romacs BIN --runtime DIR --harness DIR --output DIR --work DIR [--runs N] [modes]
BIN is the host binary, with romacs-keeper beside it; DIR for --runtime is
a tree with lisp/gnu, lisp/romacs and etc; --harness is this directory.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import threading
import time


def tcp_server(log):
    """A server that streams a line every 100 ms to its one client and logs
    when the client came and went."""
    server = socket.socket()
    server.bind(('127.0.0.1', 0))
    server.listen(4)

    def serve():
        while True:
            try:
                peer, _ = server.accept()
            except OSError:
                return
            log.append(('accept', time.time()))
            count = 0
            try:
                while True:
                    count += 1
                    peer.sendall(f'stanza {count}\n'.encode())
                    peer.settimeout(.1)
                    try:
                        if peer.recv(4096) == b'':
                            log.append(('eof', time.time()))
                            break
                    except socket.timeout:
                        pass
            except OSError as error:
                log.append((f'error {error.errno}', time.time()))
            peer.close()

    threading.Thread(target=serve, daemon=True).start()
    return server, server.getsockname()[1]


class Captures:
    """grim in a loop: what the compositor's output shows, with times."""

    def __init__(self, engine, directory):
        self.engine, self.directory, self.frames, self.stopping = engine, directory, [], False
        directory.mkdir()
        self.thread = threading.Thread(target=self.loop, daemon=True)

    def loop(self):
        index = 0
        while not self.stopping:
            path = self.directory / f'cap-{index:04d}.ppm'
            begun = time.time()
            done = subprocess.run([f'{self.engine.tools}/grim', '-t', 'ppm', str(path)], env=self.engine.env,
                                  capture_output=True, timeout=30)
            ended = time.time()
            if done.returncode == 0:
                self.frames.append([begun, ended, path])
            else:
                self.frames.append([begun, ended, None])
            index += 1

    def start(self):
        self.thread.start()
        return self

    def stop(self):
        self.stopping = True
        self.thread.join(40)

    def timeline(self, origin):
        """Runs of identical captures: (first begun, last ended, count, file, colours)."""
        runs = []
        for begun, ended, path in self.frames:
            digest = hashlib.sha1(path.read_bytes()).hexdigest() if path else 'no-capture'
            if runs and runs[-1]['sha'] == digest:
                runs[-1]['until'] = round(ended - origin, 3)
                runs[-1]['count'] += 1
                if path:
                    path.unlink()
                continue
            name = None
            colours = None
            if path:
                png = path.with_suffix('.png')
                subprocess.run(['magick', str(path), str(png)], check=False, capture_output=True,
                               env={k: v for k, v in os.environ.items() if k != 'LD_LIBRARY_PATH'})
                counted = subprocess.run(['magick', 'identify', '-format', '%k', str(path)], capture_output=True, text=True,
                                         env={k: v for k, v in os.environ.items() if k != 'LD_LIBRARY_PATH'})
                colours = int(counted.stdout) if counted.stdout.strip().isdigit() else None
                path.unlink()
                name = png.name
            runs.append({'from': round(begun - origin, 3), 'until': round(ended - origin, 3), 'count': 1,
                         'sha': digest, 'file': name, 'colours': colours})
        return runs


def alive(pid):
    try:
        status = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
        return status != 'Z'
    except (FileNotFoundError, ProcessLookupError):
        return False


def last_tick(path):
    """(count, time) of the last complete line of a ticks file."""
    try:
        lines = path.read_text().splitlines()
    except FileNotFoundError:
        return None
    for line in reversed(lines):
        parts = line.split()
        if len(parts) == 2:
            return int(parts[0]), float(parts[1])
    return None


def comms(pid):
    """How many threads of PID carry each name."""
    names = {}
    for task in Path(f'/proc/{pid}/task').iterdir():
        try:
            name = (task / 'comm').read_text().strip()
        except OSError:
            continue
        names[name] = names.get(name, 0) + 1
    return dict(sorted(names.items(), key=lambda item: (-item[1], item[0])))


def one(args, index, output):
    import parity_live
    import restart_live
    from parity_live import K, M, T, Engine, lisp_string, plist, prepare_world, private_tree, wait
    from restart_live import INIT, A_TEXT, B_TEXT, OBSERVE, Pid, edit, ready

    root, home, env = private_tree('romacs', Path(args.work), args.tools, prefix='term-keep-')
    wlr = dict(pair.split('=', 1) for pair in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split())
    if wlr:
        env.update(wlr)
        env.pop('WGPU_BACKEND', None)
    runtime = Path(args.runtime)
    newer_env = dict(env, ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14',
                     ROMACS_RUNTIME_ROOT=str(runtime), ROMACS_LISP_DIR=str(runtime / 'lisp/gnu'))
    binary, extra = args.romacs, ''
    if args.older:
        # The session runs the older build with its own runtime; the restart
        # names the newer build and gives it its runtime, as an install does.
        binary, runtime = args.older, Path(args.older_runtime)
        newer = Path(args.runtime)
        extra = (f'(setq romacs-restart-command (list {lisp_string(args.romacs)} "--loadup"))\n'
                 f'(setq romacs-restart-environment (list {lisp_string("ROMACS_RUNTIME_ROOT=" + str(newer))} '
                 f'{lisp_string("ROMACS_LISP_DIR=" + str(newer / "lisp/gnu"))}))')
    env.update(ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14',
               ROMACS_RUNTIME_ROOT=str(runtime), ROMACS_LISP_DIR=str(runtime / 'lisp/gnu'))
    (home / '.config/romacs').mkdir()
    (home / '.config/romacs/init.el').write_text(INIT.format(extra=extra, probe=lisp_string(str(parity_live.PROBE))))
    work = root / 'work'
    (work / 'a.txt').write_text(A_TEXT)
    (work / 'b.el').write_text(B_TEXT)
    tick = 'i=0; echo $$ > "$1.pid"; while :; do i=$((i+1)); echo "line $i"; echo "$i $(date +%s.%N)" >> "$1.ticks"; sleep 0.1; done\n'
    (work / 'term.sh').write_text('#!/bin/sh\n' + tick.replace('$1', str(work / 'term')))
    (work / 'term.sh').chmod(0o755)
    (work / 'agent.sh').write_text('#!/bin/sh\n' + tick.replace('$1', str(work / 'agent')))
    (work / 'agent.sh').chmod(0o755)
    peer_log = []
    server, port = tcp_server(peer_log)
    entry = {'run': index, 'renderer': env.get('WLR_RENDERER'), 'root': str(root)}
    entry['world_seconds'] = prepare_world(binary, env, work)
    if args.older:
        (root / 'work2').mkdir()
        entry['newer_world_seconds'] = None if args.unprepared else prepare_world(args.romacs, newer_env, root / 'work2')
    entry['cross_build'], entry['unprepared'], entry['extra_buffers'] = bool(args.older), args.unprepared, args.buffers
    engine = Engine('romacs', [binary, '--loadup'], root, None, env, args.timeout)
    if args.deep_runtime:
        engine.editor_runtime = root / ('deep-runtime-' + 'r' * 60) / ('s' * 40)
        engine.editor_runtime.mkdir(parents=True, mode=0o700)
    engine.start(args.tools)
    try:
        ready(engine, '(eq (frame-focus-state) t)', 'focus')
        entry['startup_seconds'] = round(engine.startup_seconds, 3)
        entry['world'] = [line for line in (root / 'engine.log').read_text(errors='replace').splitlines()
                          if 'romacs-world' in line][:2]
        before = edit(engine, root)
        if args.cat:
            # Throughput: `cat` of a big text file in a terminal, from the
            # command to its last line in the buffer.
            engine.ask('(progn (romacs-term "/bin/sh") t)')
            ready(engine, '(with-current-buffer "*romacs-term*" (> (buffer-size) 1))', 'a prompt')
            times = []
            for _ in range(args.cat_runs):
                engine.ask(f'(with-current-buffer "*romacs-term*" (romacs--term-send romacs-term-session {lisp_string("cat " + args.cat + "; echo CAT-DONE-$((1+1))" + chr(10))}) t)')
                started = time.time()
                wait(lambda: engine.ask('(with-current-buffer "*romacs-term*" (save-excursion (goto-char (point-max)) (and (search-backward "CAT-DONE-2" (max (point-min) (- (point-max) 400)) t) t)))'), 600, 'the end of cat', None, .02)
                times.append(round(time.time() - started, 3))
                engine.ask('(with-current-buffer "*romacs-term*" (romacs--term-send romacs-term-session "clear\n") t)')
                time.sleep(1)
            entry['cat_seconds'] = times
            entry['cat_counts'] = engine.ask('(and (fboundp (quote romacs--term-transport-counts)) (with-current-buffer "*romacs-term*" (and romacs-term-session (fboundp (quote romacs--term-transport-counts)) (let ((c (romacs--term-transport-counts romacs-term-session))) (list (car c) (cdr c))))))')
            return entry
        if args.buffers:
            body = ''.join(f'(defun extra-{{n}}-{i} (x) "Doc." (when x (list x {i})))\n' for i in range(200))
            for n in range(args.buffers):
                (work / f'extra-{n}.el').write_text(';;; -*- lexical-binding: t -*-\n' + body.format(n=n))
            engine.ask(f'''(progn (dotimes (n {args.buffers})
                (with-current-buffer (find-file-noselect (expand-file-name (format "extra-%d.el" n) {lisp_string(str(work))}))
                  (goto-char (point-max)) (insert ";; edited\n") (font-lock-ensure))) t)''', 120)
        # The three things to keep.  The terminal last, so its window shows.
        engine.ask(f'''(progn
          (set-process-query-on-exit-flag
           (make-process :name "agent-stub" :buffer (get-buffer-create "*agent-stub*")
                         :command (list {lisp_string(str(work / 'agent.sh'))}) :connection-type 'pipe) nil)
          (set-process-query-on-exit-flag
           (make-network-process :name "network-stub" :buffer (get-buffer-create "*network-stub*")
                                 :host "127.0.0.1" :service {port}) nil)
          t)''')
        engine.ask(f'(progn (romacs-term {lisp_string(str(work / "term.sh"))}) t)')
        wait(lambda: (work / 'term.pid').exists() and (work / 'agent.pid').exists() and peer_log, 20, 'the three stand-ins')
        ready(engine, '(with-current-buffer "*agent-stub*" (> (buffer-size) 20))', 'agent output')
        ready(engine, '(with-current-buffer "*network-stub*" (> (buffer-size) 20))', 'network output')
        time.sleep(1)
        pids = {'shell': int((work / 'term.pid').read_text()), 'agent': int((work / 'agent.pid').read_text())}
        socket_path = engine.ask('(and (romacs-keeper-attached-p) (romacs-keeper-socket))')
        entry['keeper_socket'] = {'path': socket_path, 'bytes': len(os.fsencode(socket_path)) if socket_path else None}
        if args.keeper_exec:
            # The keeper replaces itself in place while the terminal runs.
            kpid = engine.ask('(plist-get (romacs-keeper-info) :pid)')
            started = time.time()
            engine.ask('(progn (romacs--keeper-call (quote exec) (romacs-keeper-program)) t)')
            entry['keeper_exec'] = {'seconds': round(time.time() - started, 4), 'same_pid': engine.ask('(plist-get (romacs-keeper-info) :pid)') == kpid,
                                    'handles_after': engine.ask('(length (romacs-keeper-handles))'),
                                    'terminal_running_after': bool(engine.ask('(with-current-buffer "*romacs-term*" (and (romacs-term-running-p) t))'))}
            time.sleep(0.5)
        if args.refuse:
            # A successor build that reads other helper versions: the restart
            # must be refused before anything is detached, naming the terminal.
            fake = work / 'successor'
            fake.write_text('#!/bin/sh\n[ "$1" = --terminal-versions ] && echo "2 3"\n')
            fake.chmod(0o755)
            engine.ask(f'(progn (setq romacs-restart-command (list {lisp_string(str(fake))})) t)')
            engine.ask('(progn (require (quote warm-restart)) t)')
            blocked = engine.ask('(mapcar (lambda (e) (format "%s: %s" (plist-get e :name) (plist-get e :reason))) (warm-restart--entries (warm-restart-plan) (quote blocked)))')
            refusal = engine.ask('(condition-case err (progn (romacs-restart) "restarted") (error (error-message-string err)))')
            time.sleep(1.0)
            tick = last_tick(work / 'term.ticks')
            entry['refuse'] = {'helper_version': engine.ask('(with-current-buffer "*romacs-term*" (romacs--term-helper-version romacs-term-session))'),
                               'blocked': blocked, 'romacs_restart_said': refusal, 'session_alive': engine.process.poll() is None,
                               'shell_still_ticking': bool(tick and time.time() - tick[1] < 0.5),
                               'terminal_running': bool(engine.ask('(with-current-buffer "*romacs-term*" (and (romacs-term-running-p) t))'))}
            entry['refuse']['pass'] = bool(any('*romacs-term*' in b and 'helper version 1' in b and '2-3' in b for b in blocked or [])
                                           and entry['refuse']['session_alive'] and entry['refuse']['shell_still_ticking'] and entry['refuse']['terminal_running']
                                           and refusal != 'restarted')
            entry['keeper_pid'] = engine.ask('(romacs-session-pid)')
            return entry
        if args.terminals > 1:
            # More terminals, each with a full scrollback, to carry across.
            for n in range(2, args.terminals + 1):
                engine.ask(f'(progn (romacs-term "/bin/sh" {n}) (romacs--term-send romacs-term-session "seq 1 12000\n") t)')
            ready(engine, f'(with-current-buffer "*romacs-term*<{args.terminals}>" (save-excursion (goto-char (point-max)) (and (search-backward "12000" nil t) t)))', 'the scrollbacks to fill')
            engine.ask('(progn (switch-to-buffer "*romacs-term*") t)')
            time.sleep(0.5)
        helper_kill = None
        if args.kill_helper:
            ROW = '(with-current-buffer "*romacs-term*" (let (found) (dolist (r (romacs-keeper-handles) found) (when (eql (plist-get r :handle) romacs-term-handle) (setq found (list (plist-get r :pid) (format "%S" (plist-get r :status))))))))'
            helper, _ = engine.ask(ROW)
            os.kill(helper, 9)
            killed = time.time()
            wait(lambda: not engine.ask('(with-current-buffer "*romacs-term*" (and (romacs-term-running-p) t))'), 10, 'the terminal to end', None, .01)
            ended = round(time.time() - killed, 3)
            status = engine.ask(ROW)[1]
            # Its shell is on the pty the keeper holds: the session lets the
            # handle go, which hangs the shell up. Nothing is left running
            # that nobody can reach.
            shell = int((work / 'term.pid').read_text())
            wait(lambda: not alive(shell), 10, 'the shell to be hung up', None, .01)
            helper_kill = {'helper_pid': helper, 'live_terminal_ended_after_seconds': ended, 'row_status_after_kill': status,
                           'shell_hung_up_after_seconds': round(time.time() - killed, 3),
                           'handles_left': engine.ask('(length (romacs-keeper-handles))')}
        def links(pid):
            out = []
            for fd in Path(f'/proc/{pid}/fd').iterdir():
                try:
                    out.append(os.readlink(fd))
                except OSError:
                    pass
            return sorted(link for link in out if 'ptmx' in link or '/pts/' in link)
        try:
            who = engine.ask('(condition-case nil (with-current-buffer "*romacs-term*" (let (found) (dolist (r (romacs-keeper-handles) found) (when (eql (plist-get r :handle) romacs-term-handle) (setq found (list (romacs-session-pid) (plist-get r :pid) (format "%S" (plist-get r :open)))))))) (error nil))')
            entry['pty_holders'] = {'keeper': links(who[0]), 'helper': links(who[1]), 'session': links(engine.process.pid), 'open_slots': who[2],
                                    'helper_rss_kb': {k: int(v.split()[0]) for k, v in (line.split(':', 1) for line in Path(f'/proc/{who[1]}/smaps_rollup').read_text().splitlines()[1:]) if k in ('Rss', 'Pss', 'Private_Clean', 'Private_Dirty')},
                                    'keeper_rss_kb': {k: int(v.split()[0]) for k, v in (line.split(':', 1) for line in Path(f'/proc/{who[0]}/smaps_rollup').read_text().splitlines()[1:]) if k in ('Rss', 'Pss', 'Private_Clean', 'Private_Dirty')}} if who else None
        except Exception as error:
            entry['pty_holders'] = repr(error)[:200]
        entry['before'] = {
            'comm': comms(engine.process.pid),
            'plan_processes': engine.ask('(mapcar (lambda (p) (format "%S" p)) (plist-get (warm-restart-plan) :processes))'),
            'plan_dropped': engine.ask('''(let (out) (dolist (e (plist-get (warm-restart-plan) :buffers) (nreverse out))
              (unless (memq (plist-get e :strategy) '(text file)) (push (format "%S" e) out))))'''),
            'blocker': engine.ask('(format "%S" (mapcar (lambda (e) (plist-get e :name)) (warm-restart--entries (warm-restart-plan) (quote blocked))))'),
            'process_list': engine.ask('(mapcar #\'process-name (process-list))'),
            'term_handle': None,
            'messages': engine.ask('(with-current-buffer "*Messages*" (buffer-substring-no-properties (max (point-min) (- (point-max) 600)) (point-max)))'),
            'buffers': engine.ask('(mapcar #\'buffer-name (buffer-list))'),
            'threads': len(list(Path(f'/proc/{engine.process.pid}/task').iterdir())),
            'fds': len(list(Path(f'/proc/{engine.process.pid}/fd').iterdir())),
        }
        old_pid = engine.process.pid
        asking = engine.ask('(seq-filter (lambda (p) (plist-get p :query)) (plist-get (warm-restart-plan) :processes))')
        (engine.probe_dir / 'alive').unlink()
        if args.kill:
            # The crash journey: no image, no hook; a relaunch finds the keeper.
            import subprocess
            environ = dict(pair.split('=', 1) for pair in Path(f'/proc/{old_pid}/environ').read_text().split('\0') if '=' in pair)
            asking = False
            pressed = sent = time.time()
            os.kill(old_pid, 9)
            wait(lambda: engine.process.poll() is not None, engine.timeout, 'the session to die', None, .002)
            exited = time.time()
            status = engine.process.returncode
            subprocess.Popen([binary, '--loadup'], env=environ, cwd=str(root), stdin=subprocess.DEVNULL,
                             stdout=open(root / 'relaunch.log', 'w'), stderr=subprocess.STDOUT, start_new_session=True)
            wait(lambda: (engine.probe_dir / 'alive').exists(), engine.timeout * 2, 'the relaunched process init', None, .002)
            inited = time.time()
            new_pid = int((engine.probe_dir / 'pid').read_text())
            engine.process = Pid(new_pid)
            engine.processes.append(engine.process)
            wait(lambda: engine.ask('(and (get-buffer "*romacs-term*") t)'), 60, 'the terminal to come back', None, .01)
            restored = focused = time.time()
            time.sleep(1.5)
        engine.keys(M('x')) if not args.kill else None
        if not args.kill:
            ready(engine, '(minibufferp)', 'M-x prompt')
            typed = time.time()
            engine.keys(T('romacs-restart'))
            wait(lambda: engine.ask('(equal (minibuffer-contents-no-properties) "romacs-restart")'), 60, 'the typed command', None, .01)
            entry['mx_typed_seconds'] = round(time.time() - typed, 3)
            time.sleep(.5)
            captures = Captures(engine, output / f'run{index}-captures').start()
            time.sleep(.4)
            if asking:
                engine.keys(K('Return'))
                ready(engine, '(string-match-p "Restart anyway" (or (minibuffer-prompt) ""))', 'the restart question')
                engine.keys(T('yes'))
            pressed = time.time()
            engine.keys(K('Return'))
            sent = time.time()
            wait(lambda: engine.process.poll() is not None, engine.timeout, 'the session to exit', None, .002)
            exited = time.time()
            status = engine.process.returncode
            wait(lambda: (engine.probe_dir / 'alive').exists(), engine.timeout * 2, 'the new process init', None, .002)
            inited = time.time()
            new_pid = int((engine.probe_dir / 'pid').read_text())
            engine.process = Pid(new_pid)
            engine.processes.append(engine.process)
            # Asked plainly: `ready' also waits for one-shot timers, and the
            # restore's own toast holds one for two seconds.
            wait(lambda: engine.ask('(and warm-restart-last-report t)'), 60, 'the restore', None, .005)
            restored = time.time()
            wait(lambda: engine.ask('(eq (frame-focus-state) t)'), 60, 'focus in the new process', None, .005)
            focused = time.time()
            time.sleep(1.5)
            captures.stop()
        entry['asked'] = bool(asking)
        entry['times'] = {'key_sent': round(sent - pressed, 3), 'old_exit': round(exited - pressed, 3),
                          'new_init_done': round(inited - pressed, 3), 'restored': round(restored - pressed, 3),
                          'focused': round(focused - pressed, 3)}
        entry['status'], entry['old_pid'], entry['new_pid'] = status, old_pid, new_pid
        entry['screen'] = captures.timeline(pressed) if not args.kill else None
        after = plist(engine.ask(OBSERVE)) if not args.kill else {}
        entry['journey'] = 'kill -9 and relaunch' if args.kill else 'romacs-restart'
        entry['session_equal'] = {key: before[key] == after[key] for key in ('buffers', 'kill')} if not args.kill else {}
        if not args.kill:
            entry['session_equal']['windows'] = [w[:4] for w in before['windows']] == [w[:4] for w in after['windows']]
        entry['report'] = None if args.kill else engine.ask('(list :buffers (plist-get warm-restart-last-report :buffers)'
                                     ' :seconds (plist-get warm-restart-last-report :seconds)'
                                     ' :problems (mapcar (lambda (e) (format "%S" e)) (plist-get warm-restart-last-report :problems))'
                                     ' :lost (mapcar (lambda (e) (format "%S" e)) (plist-get warm-restart-last-report :lost))'
                                     ' :error (plist-get warm-restart-last-report :error))')
        now = time.time()
        died = {}
        for name, stem in (('shell', 'term'), ('agent', 'agent')):
            tick = last_tick(work / f'{stem}.ticks')
            died[name] = {'pid': pids[name], 'alive_after': alive(pids[name]),
                          'last_tick_after_key': round(tick[1] - pressed, 3) if tick else None,
                          'still_ticking': bool(tick and now - tick[1] < 1)}
        died['network'] = {'peer_log': [(what, round(when - pressed, 3)) for what, when in peer_log]}
        died['after_process_list'] = engine.ask('(mapcar #\'process-name (process-list))')
        died['after_buffers'] = engine.ask('(mapcar #\'buffer-name (buffer-list))')
        died['after_terminal_running'] = engine.ask('(and (get-buffer "*romacs-term*") (with-current-buffer "*romacs-term*" (and (romacs-term-running-p) t)))')
        # K2's shell row: the ticks went on through the restart, the pid is
        # the same, and the buffer shows what was printed meanwhile.
        ticks = [(int(a), float(b)) for a, b in (line.split() for line in (work / 'term.ticks').read_text().splitlines() if len(line.split()) == 2)]
        window = [t for t in ticks if pressed - 1 <= t[1] <= restored + 1]
        gaps = [round(b[1] - a[1], 3) for a, b in zip(window, window[1:])]
        during = [n for n, when in ticks if exited <= when <= restored]
        text = engine.ask('(and (get-buffer "*romacs-term*") (with-current-buffer "*romacs-term*" (buffer-substring-no-properties (point-min) (point-max))))') or ''
        shown = [n for n in during if f'line {n}\n' in text or text.rstrip().endswith(f'line {n}')]
        time.sleep(0.5)
        later = last_tick(work / 'term.ticks')
        text2 = engine.ask('(and (get-buffer "*romacs-term*") (with-current-buffer "*romacs-term*" (buffer-substring-no-properties (point-min) (point-max))))') or ''
        entry['k2_shell'] = {
            'pid_alive': alive(pids['shell']),
            'terminal_running': bool(died['after_terminal_running']),
            'max_tick_gap_seconds': max(gaps) if gaps else None,
            'ticks_during_restart': during,
            'ticks_during_restart_shown': shown,
            'buffer_follows_new_output': bool(later and f'line {later[0]}' in text2),
            'handle': engine.ask('(and (get-buffer "*romacs-term*") (boundp (quote romacs-term-handle)) (with-current-buffer "*romacs-term*" romacs-term-handle))'),
            'session_pid': engine.ask('(condition-case nil (progn (require (quote romacs-keeper)) (romacs-session-pid)) (error nil))'),
            'reattach_microseconds': engine.ask('(and (boundp (quote romacs-term-reattach-seconds)) (mapcar (lambda (e) (round (* 1000000 (cdr e)))) romacs-term-reattach-seconds))'),
            'transport_counts': engine.ask('(and (get-buffer "*romacs-term*") (with-current-buffer "*romacs-term*" (and romacs-term-session (fboundp (quote romacs--term-transport-counts)) (let ((c (romacs--term-transport-counts romacs-term-session))) (list (car c) (cdr c))))))'),
            'threads': len(list(Path(f'/proc/{engine.process.pid}/task').iterdir())),
            'fds': len(list(Path(f'/proc/{engine.process.pid}/fd').iterdir())),
            'comm': comms(engine.process.pid),
        }
        if helper_kill is not None:
            helper_kill['restored_buffer_says'] = text.strip()[:160]
            helper_kill['pass'] = bool('signaled 9' in helper_kill['row_status_after_kill'] and helper_kill['handles_left'] == 0
                                       and not alive(pids['shell']))
            entry['k2_helper_kill'] = helper_kill
        if args.terminals > 1:
            states = engine.ask(f'(mapcar (lambda (n) (let ((b (get-buffer (format "*romacs-term*<%d>" n)))) (and b (with-current-buffer b (list (and (romacs-term-running-p) t) (count-lines (point-min) (point-max))))))) (number-sequence 2 {args.terminals}))')
            entry['k2_terminals'] = {'count': args.terminals, 'others_running_and_lines': states,
                                     'all_back': all(state and state[0] and state[1] > 9000 for state in states)}
        entry['k2_shell']['pass'] = bool((not args.deep_runtime or (entry['keeper_socket']['bytes'] or 0) > 107)
                                         and entry['k2_shell']['pid_alive'] and entry['k2_shell']['terminal_running']
                                         and gaps and max(gaps) < 0.25 and during and shown == during
                                         and entry['k2_shell']['buffer_follows_new_output'])
        entry['keeper_pid'] = entry['k2_shell']['session_pid']
        entry['died'] = died
        state = home / '.local/state/romacs/warm-restart'
        entry['exe_after'] = after.get('exe')
        entry['successor_log'] = [line for path in sorted(state.glob('*.log'))
                                  for line in path.read_text(errors='replace').splitlines()][-40:] if state.exists() else []
    finally:
        keeper = entry.get('keeper_pid')
        if not keeper:
            try:
                keeper = engine.ask('(condition-case nil (progn (require (quote romacs-keeper)) (romacs-session-pid)) (error nil))')
            except Exception:
                keeper = None
        engine.stop()
        # The keeper outlives the session by design: end it, and its helpers with it.
        if isinstance(keeper, int) and keeper not in (engine.process.pid,):
            try:
                os.kill(keeper, 9)
            except OSError:
                pass
        server.close()
        for pid_file in ('term.pid', 'agent.pid'):
            try:
                os.kill(int((work / pid_file).read_text()), 9)
            except (OSError, ValueError):
                pass
    return entry


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--runtime', required=True)
    parser.add_argument('--harness', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', required=True)
    parser.add_argument('--runs', type=int, default=3)
    parser.add_argument('--older', help='the build the session starts in; --romacs is then the newer one')
    parser.add_argument('--older-runtime')
    parser.add_argument('--unprepared', action='store_true', help="do not prepare the newer build's world first")
    parser.add_argument('--buffers', type=int, default=0, help='extra edited Lisp files to carry')
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--cat', help='time cat of this file in a terminal instead of the restart journey')
    parser.add_argument('--cat-runs', type=int, default=3)
    parser.add_argument('--keeper-exec', action='store_true', help='exec the keeper in place before the journey')
    parser.add_argument('--refuse', action='store_true', help='a successor that reads other helper versions: the restart must be refused')
    parser.add_argument('--terminals', type=int, default=1, help='this many terminals, the extra ones with a full scrollback')
    parser.add_argument('--kill-helper', action='store_true', help='kill -9 the terminal helper first: the terminal must end, and restore as ended')
    parser.add_argument('--kill', action='store_true', help='kill -9 the session and relaunch, instead of romacs-restart')
    parser.add_argument('--deep-runtime', action='store_true', help="a session runtime directory deeper than a socket address holds")
    args = parser.parse_args()
    sys.path.insert(0, args.harness)
    import parity_live
    args.tools = parity_live.TOOLS
    output = Path(args.output)
    output.mkdir(parents=True)
    Path(args.work).mkdir(parents=True, exist_ok=True)
    report = {'romacs': args.romacs, 'runtime': args.runtime, 'load': os.getloadavg(), 'runs': []}
    for index in range(1, args.runs + 1):
        try:
            entry = one(args, index, output)
        except Exception as error:  # noqa: BLE001 - a failed run is that run's result
            import traceback
            entry = {'run': index, 'error': f'{type(error).__name__}: {error}', 'trace': traceback.format_exc()}
        report['runs'].append(entry)
        print(index, entry.get('times'), entry.get('error', ''), flush=True)
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False, default=str) + '\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
