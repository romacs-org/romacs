#!/usr/bin/env python3
"""Agent sessions (rlisp/romacs-agent.el, romacs-agent-host, the
reconnecting romacs-mcp relay) in a private headless labwc, HOME, XDG and
TMPDIR, with fake_harness.py speaking Claude Code's stream-json in place of
`claude' (no model is called).

  session    a session reads a buffer, edits it through romacs_buffer_edit,
             evaluates on the main VM, and its chat buffer shows each step;
             an approval the harness asks for is answered as the user says,
             with the tool's input written back unchanged; an interrupt ends
             a long turn; closing and showing the window keeps the session.
  survive    `romacs --stop' during a long turn, then a new server: the
             session is attached again, the turn completes in the journal
             and the chat, and the harness pid is the same; the harness's
             next call meets the new instance and retries once.
  outcome    a call running when the server is killed is answered `outcome
             unknown' and romacs_call_outcome says `unknown', and it ran
             once; a call made while no server listens is `not executed'
             and has no effect.
  budget     `(while t)' under budgetMs 1000 is quit within 1.1 s, and the
             next command is not; C-g during a 10 s agent evaluation quits
             it at once.
  unit       with the user manager, a session's unit ends a grandchild the
             harness started when the session stops.
  host       the supervisor alone: a harness writing 4 MB and exiting
             while the attached server never reads; and a harness whose
             descendant holds its stdin unread, one server blocked writing
             to it and another stealing the attachment: each time the exit
             is journaled and the supervisor ends.
  deadline   with a 2 s reply timeout: a call still running past it is
             answered `outcome unknown' (romacs_call_outcome: done, it ran
             once); a call queued behind a busy editor is withdrawn,
             answered `not executed', and never runs.
  probe      sessions (1, 10, 100): one event dispatched to one session;
             a session's call log (1, 100, 1000 calls): recording one and
             looking one up; a chat of 1, 100, 1000 tool calls: one result;
             withdrawing one call among 1, 16, 64 queued.

usage: agent_live.py --romacs BIN --output DIR [--workflow NAME]... [--tools DIR]
"""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

import parity_live
import server_live
from server_live import Server, until, lisp

HARNESS = Path(__file__).resolve().parent / 'fake_harness.py'


class Agents(Server):
    def __init__(self, args, base, name, env_extra=None):
        super().__init__(args, base, name, env_extra)
        self.calls = self.root / 'harness-calls.jsonl'
        self.env['FAKE_HARNESS_LOG'] = str(self.calls)
        self.work = self.root / 'work'

    def up(self):
        self.client('-n')
        self.pid = int(self.eval('(emacs-pid)'))
        self.eval(f"(progn (require 'romacs-agent) (setq romacs-agent-claude-program {lisp(HARNESS)}) t)")

    def start(self, prompt):
        sid = self.eval(f'(romacs-agent-id (romacs-agent-start {lisp(self.work)} {lisp_text(prompt)}))')
        return sid.strip('"')

    def s(self, sid):
        return f'(romacs-agent-get {lisp_text(sid)})'

    def send(self, sid, text):
        self.eval(f'(romacs-agent-send {self.s(sid)} {lisp_text(text)})')

    def state(self, sid):
        return self.eval(f'(romacs-agent-state {self.s(sid)})')

    def idle(self, sid, seconds=30):
        until(lambda: self.state(sid) == 'idle', seconds, f'session {sid} idle')

    def journal(self, sid):
        path = self.home / '.local/state/romacs/agents' / sid / 'journal.jsonl'
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def results(self, sid):
        return [r['out'].get('result') for r in self.journal(sid)
                if isinstance(r.get('out'), dict) and r['out'].get('type') == 'result']

    def chat(self, sid):
        return parity_live.read_sexp(self.eval(
            f'(with-current-buffer (romacs-agent-show {self.s(sid)}) (buffer-substring-no-properties (point-min) (point-max)))'))

    def harness_pid(self, sid):
        host = json.loads((self.home / '.local/state/romacs/agents' / sid / 'host.json').read_text())
        return host['harness']

    def logged(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()] if self.calls.exists() else []

    def restart(self):
        """A new server: what a user's next `romacs' starts."""
        self.client('-n')
        self.pid = int(self.eval('(emacs-pid)'))
        self.eval(f"(progn (require 'romacs-agent) (setq romacs-agent-claude-program {lisp(HARNESS)}) "
                  "(romacs-agent-restore) t)")

    def close(self):
        # Sessions started detached: end their supervisors and harnesses.
        for host in (self.home / '.local/state/romacs/agents').glob('*/host.json'):
            info = json.loads(host.read_text())
            for pid in (info.get('harness'), info.get('pid')):
                try:
                    os.kill(pid, signal.SIGKILL)
                except (ProcessLookupError, TypeError):
                    pass
        super().close()


def lisp_text(text):
    return parity_live.lisp_string(text)


def session(args, base):
    a = Agents(args, base, 'ag')
    try:
        a.up()
        sid = a.start('say hello from the fake harness')
        a.idle(sid)
        a.eval('(with-current-buffer (get-buffer-create "notes") (erase-buffer) (insert "alpha beta\\n") t)')
        a.send(sid, 'call romacs_buffer_read {"instance": "?", "name": "notes", "start": 1, "maxChars": 100}')
        a.idle(sid)
        a.send(sid, 'call romacs_buffer_edit {"instance": "?", "name": "notes", '
                    '"edits": [{"start": 1, "end": 6, "text": "ALPHA"}]}')
        a.idle(sid)
        edited = a.eval('(with-current-buffer "notes" (buffer-string))')
        a.send(sid, 'call romacs_eval {"instance": "?", "code": "(+ 20 22)"}')
        a.idle(sid)
        evaluated = a.results(sid)[-1]
        a.send(sid, 'ask touch approved.txt')
        until(lambda: a.state(sid) == 'waiting-approval', 20, 'approval raised')
        a.eval(f'(let ((s {a.s(sid)})) (romacs-agent-respond s (car (hash-table-keys (romacs-agent-approvals s))) t))')
        a.idle(sid)
        approval = next(e['answer'] for e in a.logged() if 'approval' in e)
        a.send(sid, 'slow 200')
        time.sleep(1)
        interrupted_at = time.time()
        a.eval(f'(romacs-agent-interrupt {a.s(sid)})')
        a.idle(sid)
        interrupt_s = time.time() - interrupted_at
        a.eval("(romacs--native-window 'hide)")
        a.client('-n')
        attached = a.eval(f'(and (process-live-p (romacs-agent-connection {a.s(sid)})) t)')
        chat = a.chat(sid)
        result = {'edited': edited, 'evaluated': evaluated, 'approval': approval, 'interrupt_s': round(interrupt_s, 2),
                  'attached_after_reshow': attached,
                  'chat_has': {k: k in chat for k in ('❯ say hello', 'hello from the fake harness',
                                                      '▸ mcp__romacs__romacs_buffer_edit', '✓', '⚠ Bash asks')}}
        result['ok'] = (edited == '"ALPHA beta\\n"' and evaluated == '42'
                        and approval.get('behavior') == 'allow'
                        and approval.get('updatedInput', {}).get('run_in_background') is False
                        and interrupt_s < 2 and attached == 't' and all(result['chat_has'].values()))
        return result
    finally:
        a.close()


def survive(args, base):
    a = Agents(args, base, 'sv')
    try:
        a.up()
        sid = a.start('say ready')
        a.idle(sid)
        a.send(sid, 'call romacs_eval {"instance": "?", "code": "(emacs-pid)"}')
        a.idle(sid)
        pid_before = a.harness_pid(sid)
        a.send(sid, 'slow 60')
        time.sleep(1)
        a.client('--stop=none')
        until(lambda: not Path(f'/proc/{a.pid}').exists(), 15, 'server gone')
        time.sleep(1)
        a.restart()
        until(lambda: 'slow done' in a.results(sid), 30, 'the turn completed')
        a.idle(sid)
        harness_alive = Path(f'/proc/{pid_before}').exists()
        a.send(sid, 'call romacs_eval {"instance": "?", "code": "(emacs-pid)"}')
        a.idle(sid)
        new_pid = a.results(sid)[-1]
        retries = [e for e in a.logged() if e.get('tool') == 'romacs_eval' and 'mismatch' in e.get('text', '')]
        chat = a.chat(sid)
        result = {'harness_pid_same': a.harness_pid(sid) == pid_before and harness_alive,
                  'eval_after_restart': new_pid, 'server_pid': a.pid, 'instance_retries': len(retries),
                  'chat_has_turn_end': 'line 60' in chat and 'slow done' not in chat or 'done' in chat}
        # The harness ended (stopped), then resumed on its conversation.
        a.eval(f'(romacs-agent-stop {a.s(sid)})')
        until(lambda: a.state(sid) in ('ended', 'detached'), 15, 'session ended')
        a.eval(f'(romacs-agent-resume {a.s(sid)})')
        until(lambda: a.eval(f'(and (process-live-p (romacs-agent-connection {a.s(sid)})) t)') == 't', 15, 'resumed')
        a.send(sid, 'say resumed')
        a.idle(sid)
        resumed = [e for e in a.logged() if 'started' in e][-1]
        result['resumed'] = '--resume' in resumed['argv'] and resumed['started'] == sid and a.results(sid)[-1] == 'resumed'
        result['ok'] = result['harness_pid_same'] and new_pid == str(a.pid) and len(retries) <= 1 and result['resumed']
        return result
    finally:
        a.close()


def outcome(args, base):
    a = Agents(args, base, 'oc')
    try:
        a.up()
        sid = a.start('say ready')
        a.idle(sid)
        effects = a.root / 'effects.txt'
        a.send(sid, 'call romacs_eval {"instance": "?", "code": '
                    + json.dumps(f'(progn (append-to-file "ran\\n" nil {lisp(effects)}) (sleep-for 4) 1)')
                    + ', "budgetMs": 0}')
        until(lambda: effects.exists(), 20, 'the call ran')
        os.kill(a.pid, signal.SIGKILL)
        until(lambda: not Path(f'/proc/{a.pid}').exists(), 10, 'server killed')
        a.restart()
        until(lambda: any('outcome_of' in e for e in a.logged()), 30, 'outcome asked')
        asked = next(e for e in a.logged() if 'outcome_of' in e)['outcome']
        a.idle(sid)
        ran_once = effects.read_text().count('ran') == 1
        # A call made while no server listens.
        late = a.root / 'late.txt'
        a.send(sid, 'later 3 call romacs_eval {"instance": "?", "code": '
                    + json.dumps(f'(append-to-file "late\\n" nil {lisp(late)})') + '}')
        time.sleep(.5)
        a.client('--stop=none')
        until(lambda: not Path(f'/proc/{a.pid}').exists(), 15, 'server gone')
        until(lambda: sum(1 for e in a.logged() if e.get('tool') == 'romacs_eval' and 'not executed' in e.get('text', '')) >= 1,
              20, 'not executed while down')
        not_executed = not late.exists()
        a.restart()
        result = {'outcome_after_kill': asked, 'ran_once': ran_once, 'not_executed_had_no_effect': not_executed}
        result['ok'] = '"unknown"' in asked and ran_once and not_executed
        return result
    finally:
        a.close()


def budget(args, base):
    a = Agents(args, base, 'bd')
    try:
        a.up()
        sid = a.start('say ready')
        a.idle(sid)
        a.send(sid, 'call romacs_eval {"instance": "?", "code": "(while t)", "budgetMs": 1000}')
        a.idle(sid, 20)
        spent = next(e for e in reversed(a.logged()) if e.get('tool') == 'romacs_eval')
        log = (a.home / '.local/state/romacs/agents' / sid / 'calls.jsonl').read_text().splitlines()
        records = [json.loads(line) for line in log]
        server_ms = max((r.get('ms', 0) for r in records if r.get('tool') == 'romacs_eval'), default=None)
        # A call that ends before its 300 ms budget; the budget then passes
        # while the next command, a second long, runs: it must not be quit.
        a.send(sid, 'call romacs_eval {"instance": "?", "code": "1", "budgetMs": 300}')
        a.idle(sid)
        after = a.eval('(let ((end (+ (float-time) 1)) (n 0)) (while (< (float-time) end) (setq n (1+ n))) (if (> n 0) 2000000 0))')
        a.keyboard_ready()
        time.sleep(1.5)  # the readiness probe's C-g presses, all taken
        mark = a.root / 'running.txt'
        busy = f'(progn (write-region "x" nil {lisp(mark)}) (let ((end (+ (float-time) 10))) (while (< (float-time) end))) 1)'
        a.send(sid, 'call romacs_eval {"instance": "?", "code": ' + json.dumps(busy) + ', "budgetMs": 0}')
        until(lambda: mark.exists(), 20, 'agent eval running')
        time.sleep(.3)
        pressed = time.time()
        a.keys('-M', 'ctrl', '-k', 'g', '-m', 'ctrl')
        until(lambda: any(e.get('tool') == 'romacs_eval' and e['t'] > pressed for e in a.logged()), 15, 'eval quit')
        quit = next(e for e in reversed(a.logged()) if e.get('tool') == 'romacs_eval')
        quit['seconds'] = quit['t'] - pressed
        result = {'budget_seconds': round(spent['seconds'], 3), 'budget_server_ms': server_ms, 'budget_text': spent['text'][:80], 'next_command': after,
                  'cg_seconds': round(quit['seconds'], 3), 'cg_text': quit['text'][:80]}
        result['ok'] = (server_ms is not None and server_ms < 1100 and 'Quit' in spent['text'] and after == '2000000'
                        and quit['seconds'] < .5 and 'Quit' in quit['text'])
        return result
    finally:
        a.close()


def host(args, base):
    """The supervisor alone: a harness that writes 4 MB and exits 3, an
    attached server that never reads. The exit is journaled and the
    supervisor ends: a stalled reader is detached, not waited on."""
    import socket as sockets
    binary = Path(args.romacs).with_name('romacs-agent-host')
    root = Path(os.environ.get('XDG_RUNTIME_DIR', '/tmp')) / f'anh{os.getpid()}'
    root.mkdir(mode=0o700)
    sock, journal_dir = root / 'host.sock', base / 'host-session'
    process = subprocess.Popen([str(binary), '--dir', str(journal_dir), '--socket', str(sock), '--',
                                'sh', '-c', 'sleep 0.5; yes line | head -c 4000000; exit 3'],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        until(lambda: sock.exists(), 10, 'supervisor listening')
        reader = sockets.socket(sockets.AF_UNIX, sockets.SOCK_STREAM)
        reader.connect(str(sock))
        reader.sendall(b'{"attach":{"from":1}}\n')
        started = time.time()
        process.wait(30)
        seconds = time.time() - started
        last = json.loads((journal_dir / 'journal.jsonl').read_text().splitlines()[-1])
        reader.close()
        flood = {'seconds': round(seconds, 2), 'exit_record': last.get('exit'), 'ok': last.get('exit') == 3 and seconds < 20}
        # A harness whose descendant holds its stdin unread: server A blocks
        # writing to it; server B steals the attachment and writes, waiting
        # behind A; only then does the harness exit. Its exit must still be
        # journaled and the supervisor end.
        import threading
        second, go = base / 'host-stuck', base / 'host-go'
        process = subprocess.Popen([str(binary), '--dir', str(second), '--socket', str(sock), '--',
                                    'sh', '-c', f'(sleep 30 <&0 >/dev/null &); while [ ! -e {go} ]; do sleep 0.05; done; exit 4'],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        until(lambda: sock.exists(), 10, 'second supervisor listening')

        def quietly(work):
            # A connection the supervisor's end closes is no failure here.
            def run():
                try:
                    work()
                except OSError:
                    pass
            threading.Thread(target=run, daemon=True).start()

        def reading():
            # A server that reads what it is sent (it is not a stalled one).
            connection = sockets.socket(sockets.AF_UNIX, sockets.SOCK_STREAM)
            connection.connect(str(sock))
            quietly(lambda: [None for _ in iter(lambda: connection.recv(65536), b'')])
            return connection
        first = reading()
        first.sendall(b'{"attach":{"from":1}}\n')
        line = json.dumps({'write': 'x' * 65536}).encode() + b'\n'
        quietly(lambda: [first.sendall(line) for _ in range(8)])
        time.sleep(1)  # A's writes fill the harness's stdin and block there
        thief = reading()
        thief.sendall(b'{"attach":{"from":1,"steal":true}}\n' + json.dumps({'write': 'after'}).encode() + b'\n')
        time.sleep(1)  # B's write waits behind A's
        go.touch()
        started = time.time()
        try:
            process.wait(15)
        except subprocess.TimeoutExpired:
            pass
        stuck_seconds = time.time() - started
        last = json.loads((second / 'journal.jsonl').read_text().splitlines()[-1])
        first.close()
        thief.close()
        stuck = {'seconds': round(stuck_seconds, 2), 'exit_record': last.get('exit'), 'supervisor_ended': process.poll() is not None,
                 'ok': last.get('exit') == 4 and process.poll() is not None and stuck_seconds < 10}
        return {'flood': flood, 'stuck_stdin': stuck, 'ok': flood['ok'] and stuck['ok']}
    finally:
        if process.poll() is None:
            process.kill()
        for leftover in (sock,):
            if leftover.exists():
                leftover.unlink()
        root.rmdir()


def deadline(args, base):
    a = Agents(args, base, 'dl')
    try:
        a.up()
        a.eval('(setq romacs-agent-reply-timeout 2)')
        sid = a.start('say ready')
        a.idle(sid)
        ran = a.root / 'ran.txt'
        a.send(sid, 'call romacs_eval {"instance": "?", "code": '
                    + json.dumps(f'(progn (append-to-file "ran\\n" nil {lisp(ran)}) (sleep-for 5) 1)') + ', "budgetMs": 0}')
        a.idle(sid, 40)
        running = next(e for e in reversed(a.logged()) if e.get('tool') == 'romacs_eval')
        asked = next((e['outcome'] for e in reversed(a.logged()) if 'outcome_of' in e), '')
        queued = a.root / 'queued.txt'
        # The harness calls a second after it is asked, while the editor is
        # busy with a client's 6 s evaluation: the call waits in the queue.
        a.send(sid, 'later 1 call romacs_eval {"instance": "?", "code": '
                    + json.dumps(f'(append-to-file "q\\n" nil {lisp(queued)})') + '}')
        busy = subprocess.Popen([a.args.romacs, '--server', a.name, '-e', '(progn (sleep-for 6) t)'], env=a.env,
                                cwd=a.work, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        busy.wait(30)
        a.idle(sid, 40)
        withdrawn = [e for e in a.logged() if e.get('tool') == 'romacs_eval' and 'withdrawn' in e.get('text', '')]
        result = {'running_text': running['text'][:120], 'running_seconds': round(running['seconds'], 2),
                  'outcome': asked[:120], 'ran_once': ran.read_text().count('ran') == 1,
                  'withdrawn': len(withdrawn), 'queued_ran': queued.exists()}
        result['ok'] = ('outcome unknown' in running['text'] and running['seconds'] < 4 and '"done"' in asked
                        and result['ran_once'] and withdrawn and not queued.exists())
        return result
    finally:
        a.close()


PROBE = '''(let ((times nil))
  ;; One event dispatched to one session among n.
  (dolist (n '(1 10 100))
    (clrhash romacs-agent--sessions)
    (dotimes (i n)
      (let ((s (romacs-agent--make :id (format "%08d-probe" i) :harness 'claude :title "p")))
        (puthash (romacs-agent-id s) s romacs-agent--sessions)))
    (let* ((s (romacs-agent-get "00000000-probe"))
           (line (json-parse-string "{\\"seq\\": 1, \\"out\\": {\\"type\\": \\"result\\", \\"total_cost_usd\\": 0}}"
                                    :object-type 'hash-table :null-object :null :false-object :false))
           (start (float-time)))
      (dotimes (k 100) (puthash "seq" (+ 2 k) line) (romacs-agent--record s line))
      (push (list 'sessions n 'dispatch-us (/ (* 1e6 (- (float-time) start)) 100)) times)))
  (clrhash romacs-agent--sessions)
  ;; A session's call log at n calls: recording one (appended to disk) and
  ;; looking one up.
  (dolist (n '(1 100 1000))
    (let ((sid (format "probe-calls-%d" n)))
      (make-directory (romacs-agent-tools--dir sid) t)
      (remhash sid romacs-agent-tools--calls)
      (dotimes (i n) (romacs-agent-tools--note sid (format "r:%d" i) (romacs-mcp--object "state" "done")))
      (let ((start (float-time)))
        (dotimes (k 100) (romacs-agent-tools--note sid (format "r:x%d" k) (romacs-mcp--object "state" "done")))
        (push (list 'calls n 'note-us (/ (* 1e6 (- (float-time) start)) 100)) times))
      (let ((start (float-time)))
        (dotimes (_ 1000) (gethash "r:x99" (romacs-agent-tools--table sid)))
        (push (list 'calls n 'lookup-us (/ (* 1e6 (- (float-time) start)) 1000)) times))))
  ;; A relay withdrawing one call among n queued (romacs-mcp--withdraw).
  (dolist (n '(1 16 64))
    (let ((romacs-mcp--queue nil) (romacs-mcp--queued (make-hash-table :test #'equal)))
      (dotimes (i n)
        (let ((request (list :peer 'probe-peer :message (romacs-mcp--object "id" i) :cancelled nil)))
          (setq romacs-mcp--queue (nconc romacs-mcp--queue (list request)))
          (puthash (cons 'probe-peer i) request romacs-mcp--queued)))
      (cl-letf (((symbol-function 'romacs-mcp--send) #'ignore))
        (let ((start (float-time)))
          (romacs-mcp--withdraw 'probe-peer "probe" (1- n))
          (push (list 'withdraw n 'us (* 1e6 (- (float-time) start))) times)))))
  ;; A chat with n tool calls: the last one's result rendered.
  (dolist (n '(1 100 1000))
    (let* ((s (romacs-agent--make :id (format "probe-chat-%d" n) :harness 'claude :title (format "probe %d" n)
                                  :dir temporary-file-directory)))
      (romacs-agent-show s)
      (dotimes (i n)
        (romacs-agent-ui--render s (list :type 'tool :state 'started :id (format "t%d" i) :name "Bash" :seq (1+ i) :last t)))
      (let ((start (float-time)))
        (romacs-agent-ui--render s (list :type 'tool :state 'completed :id "t0" :output "ok" :seq (+ 2 n) :last t))
        (push (list 'chat n 'complete-us (* 1e6 (- (float-time) start))) times))
      (kill-buffer (romacs-agent-buffer s))))
  (prin1-to-string (nreverse times)))'''


def probe(args, base):
    a = Agents(args, base, 'pb')
    try:
        a.up()
        rows = parity_live.read_sexp(parity_live.read_sexp(a.eval(PROBE)))
        table = [{'what': str(w), 'n': n, 'op': str(op), 'us': round(us, 1)} for w, n, op, us in rows]
        get = lambda what, n, op: next(r['us'] for r in table if (r['what'], r['n'], r['op']) == (what, n, op))
        flat = get('sessions', 100, 'dispatch-us') < 5 * get('sessions', 1, 'dispatch-us') + 50 \
            and get('calls', 1000, 'note-us') < 5 * get('calls', 1, 'note-us') + 50 \
            and get('calls', 1000, 'lookup-us') < 5 * get('calls', 1, 'lookup-us') + 5 \
            and get('chat', 1000, 'complete-us') < 5 * get('chat', 1, 'complete-us') + 200 \
            and get('withdraw', 64, 'us') < 5 * get('withdraw', 1, 'us') + 50
        return {'table': table, 'ok': flat}
    finally:
        a.close()


def unit(args, base):
    bus = os.environ.get('DBUS_SESSION_BUS_ADDRESS')
    if not bus:
        return {'ok': None, 'skipped': 'no user bus'}
    a = Agents(args, base, f'au{os.getpid()}', {'DBUS_SESSION_BUS_ADDRESS': bus, 'ROMACS_SERVER_SLICE': 'romacs-build.slice'})
    a.env['WAYLAND_DISPLAY'] = str(a.runtime / a.env['WAYLAND_DISPLAY'])
    a.env['XDG_RUNTIME_DIR'] = os.environ.get('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}')
    a.unit = f'romacs-server-{a.name}'
    try:
        a.up()
        sid = a.start('say ready')
        a.idle(sid)
        unit_name = a.eval(f'(romacs-agent-unit {a.s(sid)})').strip('"')
        trace = a.root / 'grandchild.txt'
        a.send(sid, f'spawn {trace}')
        a.idle(sid)
        grandchild = next(e['grandchild'] for e in a.logged() if 'grandchild' in e)
        a.eval(f'(romacs-agent-stop {a.s(sid)})')
        until(lambda: not Path(f'/proc/{grandchild}').exists(), 15, 'grandchild ended with the unit')
        size = trace.stat().st_size
        time.sleep(1)
        result = {'unit': unit_name, 'grandchild_gone': not Path(f'/proc/{grandchild}').exists(),
                  'stopped_writing': trace.stat().st_size == size}
        result['ok'] = unit_name.startswith('romacs-agent-') and result['grandchild_gone'] and result['stopped_writing']
        a.client('--stop=none')
        return result
    finally:
        subprocess.run(['systemctl', '--user', 'stop', a.unit], capture_output=True)
        a.close()


WORKFLOWS = {'host': host, 'deadline': deadline, 'session': session, 'survive': survive, 'outcome': outcome, 'budget': budget, 'unit': unit, 'probe': probe}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--emacsclient', default='emacsclient')
    parser.add_argument('--workflow', action='append', choices=sorted(WORKFLOWS))
    args = parser.parse_args()
    out = Path(args.output)
    out.mkdir(parents=True, exist_ok=False)
    results = {}
    for name in args.workflow or ['host', 'session', 'survive', 'outcome', 'budget', 'deadline', 'probe']:
        try:
            results[name] = WORKFLOWS[name](args, out)
        except Exception as error:  # a workflow's failure is its result
            results[name] = {'ok': False, 'error': repr(error)}
        print(name, json.dumps(results[name]), flush=True)
    (out / 'summary.json').write_text(json.dumps(results, indent=2))
    return 0 if all(r.get('ok') is not False for r in results.values()) else 1


if __name__ == '__main__':
    sys.exit(main())
