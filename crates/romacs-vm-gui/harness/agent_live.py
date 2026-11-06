#!/usr/bin/env python3
"""Agent sessions (rlisp/romacs-agent.el, romacs-agent-host, the
reconnecting romacs-mcp relay) in a private headless labwc, HOME, XDG and
TMPDIR, with fake_harness.py speaking Claude Code's stream-json in place of
`claude' (no model is called).

  providers  two instances of one harness: each harness gets its own
             CLAUDE_CONFIG_DIR (the default instance none); a resume after a
             restart uses the recorded one; a missing directory is refused.
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
  mcp_tour   every Romacs MCP tool called once through romacs_mcp_client, as
             an agent's client calls them: the server's instructions name
             the entry points, romacs_help lists the groups, observe gives a
             buffer summary by default and names conversations only when
             asked.
  instances  T3 Code's provider instances as a source (`t3' in the providers
             list): capabilities list them with their sources, a T3 account
             overlay is disabled with why, a T3 id starts its instance.
  rpc        a harness added by its driver alone (fakerpc.el, fake_rpc.py):
             a persistent JSON-RPC harness with startup steps (initialize,
             session/new or session/load), approvals, a server restart in
             the middle of its startup, a stop and resume, and its history
             listed, viewed and imported, one conversation id in two
             configuration directories two sessions.
  logging    a server keeps its own session record: errors with their
             functions, slow commands, its end.
  home_keys  RET on the home page's other lines, real keys: a project
             opens its newest, "more" shows every transcript, the help
             line says how to reach a row.
  start_picker  a session started by real keys through the palette of
             instances and models, with an effort; the first session's
             models kept once.
  environment  a harness sees none of the editor's own launch variables.
  unicode    non-ASCII (’, é, CJK, emoji, a combining mark, a raw byte)
             through prompts, replies, the journal, titles, tools, Codex
             and history.
  launcher   an editor started with no --server, restarted with its
             sessions detached: the next one serves the same agents'
             endpoint at once, sessions pinned to an ended editor's
             per-process path reach it by a link, stale ones go.
  composer   real keys typed into a chat's composer: before the first
             prompt, after a turn, DEL, after M-> and a recenter in a long
             wrapped transcript; no command signals.
  history    the harnesses' own transcripts (fixtures in private
             configuration directories) listed per project, opened
             read-only, resumed through the harness's own resume, and
             never written.
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
from romacs_mcp_client import RomacsMcp
import server_live
from server_live import Server, until, lisp

HARNESS = Path(__file__).resolve().parent / 'fake_harness.py'
CODEX = Path(__file__).resolve().parent / 'fake_codex.py'
RPC = Path(__file__).resolve().parent / 'fake_rpc.py'
RPC_DRIVER = Path(__file__).resolve().parent / 'fakerpc.el'


class Agents(Server):
    def __init__(self, args, base, name, env_extra=None):
        super().__init__(args, base, name, env_extra)
        self.calls = self.root / 'harness-calls.jsonl'
        self.env['FAKE_HARNESS_LOG'] = str(self.calls)
        self.work = self.root / 'work'
        # The default instance's directory, as a user who has run Claude Code has.
        (self.home / '.claude').mkdir()

    def up(self):
        self.client('-n')
        self.pid = int(self.eval('(emacs-pid)'))
        self.eval(f"(progn (require 'romacs-agent) (setq romacs-agent-claude-program {lisp(HARNESS)}) t)")

    def start(self, prompt, provider=None):
        named = f' nil {lisp_text(provider)}' if provider else ''
        sid = self.eval(f'(romacs-agent-id (romacs-agent-start {lisp(self.work)} {lisp_text(prompt)}{named}))')
        return sid.strip('"')

    def providers(self, entries):
        """Set `romacs-agent-providers' to ENTRIES: (name, directory) pairs,
        or (name, directory, program, arguments)."""
        def entry(e):
            if len(e) == 2:
                return f'(cons {lisp_text(e[0])} {lisp_text(str(e[1]))})'
            return (f'(list {lisp_text(e[0])} :config-dir {lisp_text(str(e[1]))} :program {lisp_text(str(e[2]))} '
                    f':arguments (list {" ".join(lisp_text(a) for a in e[3])}))')
        self.eval('(setq romacs-agent-providers (list ' + ' '.join(entry(e) for e in entries) + '))')

    def persist(self):
        """This tree's harness settings in the init file too, so a restarted
        Romacs (which runs the init again) has them."""
        home = self.home / '.config/codex-second'
        home.mkdir(parents=True, exist_ok=True)
        with open(self.home / '.config/romacs/init.el', 'a') as init:
            init.write(f"""
(setq romacs-agent-claude-program {lisp(HARNESS)}
      romacs-agent-providers (list (cons "claude" "~/.claude")
                                   (list "codex" :config-dir {lisp_text(str(home))} :program {lisp(CODEX)} :arguments nil)))
""")

    def codex_instance(self):
        """A Codex instance running fake_codex.py, with its own CODEX_HOME
        (not Codex's default ~/.codex, which leaves the variable unset)."""
        home = self.home / '.config/codex-second'
        home.mkdir(parents=True, exist_ok=True)
        self.providers([('claude', '~/.claude'), ('codex', home, CODEX, [])])
        return home

    def codex_runs(self):
        return [e for e in self.logged() if 'codex' in e]

    def tool_results(self):
        """The fake harness's tool calls: (tool, text, error) in order."""
        return [(e['tool'], e['text'], e['error']) for e in self.logged() if 'tool' in e and 'repeated' not in e]

    def config_dirs(self, sid):
        """CLAUDE_CONFIG_DIR of each harness started for SID, in order."""
        return [e.get('config_dir') for e in self.logged() if e.get('started') == sid]

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
                # Only while the pid is still this tree's process: an ended
                # one's number may be another's now.
                try:
                    if str(self.root) in Path(f'/proc/{pid}/cmdline').read_bytes().decode(errors='replace') \
                            or 'romacs-agent-host' in Path(f'/proc/{pid}/cmdline').read_bytes().decode(errors='replace'):
                        os.kill(pid, signal.SIGKILL)
                except (OSError, TypeError):
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
                  'chat_has': {k: k in chat for k in ('> say hello', 'hello from the fake harness',
                                                      '✓ romacs_buffer_edit', '✓', '◆ Bash asks')}}
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


def providers(args, base):
    """Two instances of one harness, as T3 Code runs "Claude" and a second
    account: each harness gets its instance's CLAUDE_CONFIG_DIR, the default
    instance none (the editor's own value is not passed on); a session
    resumed after a restart runs in the directory it recorded, though the
    instance now names another; a missing directory is refused clearly."""
    a = Agents(args, base, 'pv', {'CLAUDE_CONFIG_DIR': '/inherited/claude'})
    try:
        a.up()
        second, moved = a.home / '.config/claude/second', a.home / '.config/claude/moved'
        second.mkdir(parents=True)
        moved.mkdir(parents=True)
        a.providers([('claude', '~/.claude'), ('claude2', second)])
        first = a.start('say one')
        other = a.start('say two', 'claude2')
        a.idle(first)
        a.idle(other)
        started = {'default': a.config_dirs(first), 'claude2': a.config_dirs(other)}
        # The home page names each session's instance.
        home = parity_live.read_sexp(a.eval('(with-temp-buffer (romacs-agents-mode) (romacs-agent-ui--home) '
                                            '(buffer-substring-no-properties (point-min) (point-max)))'))
        shown = sorted({p for p in ('claude2', 'claude') if any(line.split('  ')[-1].startswith(p) or f' {p} ·' in line or line.rstrip().endswith(p) for line in home.splitlines())})
        a.eval(f'(romacs-agent-stop {a.s(other)})')
        until(lambda: a.state(other) in ('ended', 'detached'), 15, 'session ended')
        a.client('--stop=none')
        until(lambda: not Path(f'/proc/{a.pid}').exists(), 15, 'server gone')
        a.restart()
        a.providers([('claude', '~/.claude'), ('claude2', moved)])
        a.eval(f'(romacs-agent-resume {a.s(other)})')
        until(lambda: a.eval(f'(and (process-live-p (romacs-agent-connection {a.s(other)})) t)') == 't', 15, 'resumed')
        a.send(other, 'say resumed')
        a.idle(other)
        resumed = [e for e in a.logged() if e.get('started') == other][-1]
        a.providers([('claude', '~/.claude'), ('claude3', a.home / 'missing')])
        refused = a.eval(f'(let ((n (hash-table-count romacs-agent--sessions))) (condition-case err '
                         f'(romacs-agent-start {lisp(a.work)} nil nil "claude3") '
                         f'(user-error (list (cadr err) (= n (hash-table-count romacs-agent--sessions))))))')
        # The default directory, named and missing, is refused too.
        (a.home / '.claude').rmdir()
        default_missing = a.eval(f'(condition-case err (romacs-agent-start {lisp(a.work)} nil nil "claude") '
                                 '(user-error (cadr err)))')
        result = {'started': started, 'dashboard': shown, 'resumed_config_dir': resumed.get('config_dir'),
                  'resumed_with': '--resume' in resumed['argv'], 'missing': refused, 'default_missing': default_missing}
        result['ok'] = (started == {'default': [None], 'claude2': [str(second)]}
                        and sorted(shown) == ['claude', 'claude2']
                        and resumed.get('config_dir') == str(second) and result['resumed_with']
                        and 'does not exist' in refused and str(a.home / 'missing') in refused and refused.endswith(' t)')
                        and f'{a.home}/.claude does not exist' in default_missing)
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
        # Under a second instance: the unit's harness gets its directory.
        second = a.home / '.config/claude/second'
        second.mkdir(parents=True)
        a.providers([('claude', '~/.claude'), ('claude2', second)])
        sid = a.start('say ready', 'claude2')
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
                  'stopped_writing': trace.stat().st_size == size, 'config_dirs': a.config_dirs(sid)}
        result['ok'] = (unit_name.startswith('romacs-agent-') and result['grandchild_gone'] and result['stopped_writing']
                        and result['config_dirs'] == [str(second)])
        a.client('--stop=none')
        return result
    finally:
        subprocess.run(['systemctl', '--user', 'stop', a.unit], capture_output=True)
        a.close()

# Every Romacs MCP tool, once, with a check of its result: (tool, arguments,
# check).  `tools' (fake harness) and `real' (Claude Code) both run it; a
# tool the endpoint offers that is missing here fails the tour.
# Tools the tour leaves to their own workflows: a restart ends the tour's
# server (`restart'); gnosis's tools need its fixture database (`gnosis').
ELSEWHERE = ['romacs_restart_request']


def tour(work, delegate_target):
    page = str(work / 'tour.txt')
    return [
        ('romacs_identity', {}, lambda t: '"instance"' in t),
        ('romacs_eval', {'code': '(+ 1 2)'}, lambda t: t.strip() == '3'),
        ('romacs_help', {}, lambda t: t.startswith('core:') and 'romacs_delegate_task' in t),
        ('romacs_buffer_list', {'offset': 0, 'limit': 32}, lambda t: '"notes"' in t),
        ('romacs_buffer_read', {'name': 'notes', 'start': 1, 'maxChars': 100}, lambda t: 'alpha beta' in t),
        ('romacs_buffer_edit', {'name': 'notes', 'edits': [{'start': 1, 'end': 6, 'text': 'ALPHA'}]}, lambda t: '"tick"' in t),
        ('romacs_visit', {'path': page}, lambda t: '"tour.txt"' in t),
        ('romacs_file_write', {'path': page, 'text': 'written by the tour\n', 'save': True}, lambda t: '"saved":true' in t),
        ('romacs_observe', {'maxRows': 5}, lambda t: len(t) > 2),
        ('romacs_send_keys', {'keys': 'C-f'}, lambda t: True),
        ('romacs_wait_idle', {'timeout': 5}, lambda t: True),
        ('romacs_call_outcome', {'id': 1}, lambda t: '"outcome"' in t),
        ('romacs_orchestrator_capabilities', {}, lambda t: '"codex"' in t and '"providers"' in t),
        ('romacs_delegate_task', {'task': delegate_target['task'], 'role': 'review', 'mode': 'wait',
                                  'target': delegate_target['target'], 'clientRequestId': 'tour-review'},
         lambda t: '"completed"' in t and 'APPROVE' in t),
        ('romacs_task_status', {'taskId': '<the taskId romacs_delegate_task returned>'}, lambda t: '"completed"' in t),
        ('romacs_task_cancel', {'taskId': '<the same taskId>'}, lambda t: '"cancelled"' in t),
    ]


DIFF = """--- a/greet.py
+++ b/greet.py
@@ -1,2 +1,2 @@
 def greet(name):
-    return 'hello ' + name
+    return f'hello {name}'
"""

REVIEW = ('Review only; do not edit or run anything. Is this diff correct and behaviour-preserving? '
          'Reply with VERDICT: APPROVE or VERDICT: CHANGES and one line why.\n' + DIFF)


def offered(a):
    """The tools the session's endpoint offers, through the harness MCP client."""
    with RomacsMcp.for_server(a, agent_tools=True) as mcp:
        return mcp.tools()


def tour_setup(a):
    a.eval('(with-current-buffer (get-buffer-create "notes") (erase-buffer) (insert "alpha beta\\n") t)')
    (a.work / 'tour.txt').write_text('before\n')


def tools(args, base):
    """The fake harness calls every Romacs MCP tool once and each result
    is checked; delegation runs a fake Codex child."""
    a = Agents(args, base, 'tl')
    try:
        a.up()
        a.codex_instance()
        tour_setup(a)
        sid = a.start('say ready')
        a.idle(sid)
        steps = tour(a.work, {'task': 'review this', 'target': {'provider': 'codex'}})
        task_id = None
        for tool, arguments, _ in steps:
            if 'taskId' in arguments:
                arguments = {'taskId': task_id}
            # No instance: it is optional, and the tour shows each tool works without one.
            a.send(sid, f'call {tool} ' + json.dumps(arguments))
            a.idle(sid, 60)
            if tool == 'romacs_delegate_task':
                task_id = json.loads(a.tool_results()[-1][1])['taskId']
        results = a.tool_results()[-len(steps):]
        checked = {tool: (name == tool and not error and check(text))
                   for (tool, _, check), (name, text, error) in zip(steps, results)}
        missing = sorted(set(offered(a)) - set(checked) - set(ELSEWHERE) - {t for t in offered(a) if t.startswith('gnosis_')})
        result = {'checked': checked, 'not_in_tour': missing,
                  'notes': a.eval('(with-current-buffer "notes" (buffer-string))'),
                  'file': (a.work / 'tour.txt').read_text(),
                  'failed': {t: x[:200] for (t, _, _), (_, x, e) in zip(steps, results) if not checked.get(t)}}
        result['ok'] = all(checked.values()) and not missing and result['file'] == 'written by the tour\n'
        return result
    finally:
        a.close()


def delegate(args, base):
    """Delegation at T3 Code's parity, a fake Claude parent and fake Codex
    children: wait (with CODEX_HOME, model and effort passed), async with a
    wake carrying the result, the same clientRequestId finding its task, a
    follow-up continuing the child's thread, cancel, and a wake missed while
    the server was down delivered by the next one."""
    a = Agents(args, base, 'dg')
    try:
        a.up()
        home = a.codex_instance()
        parent = a.start('say ready')
        a.idle(parent)

        def delegate_call(arguments):
            a.send(parent, 'call romacs_delegate_task ' + json.dumps(arguments))
            a.idle(parent, 60)
            return json.loads(a.tool_results()[-1][1])

        waited = delegate_call({'task': 'say reviewed', 'mode': 'wait', 'role': 'review', 'clientRequestId': 'w1',
                                'target': {'provider': 'codex', 'model': 'm-test', 'options': {'reasoningEffort': 'low'}}})
        run = a.codex_runs()[-1]
        again = delegate_call({'task': 'say other', 'mode': 'wait', 'clientRequestId': 'w1', 'target': {'provider': 'codex'}})
        asynced = delegate_call({'task': 'slow 1', 'target': {'provider': 'codex'}, 'clientRequestId': 'a1'})
        until(lambda: any(asynced['taskId'] in e.get('wake', '') for e in a.logged()), 30, 'parent woken')
        wake = next(e['wake'] for e in a.logged() if asynced['taskId'] in e.get('wake', ''))
        a.idle(parent)
        a.eval(f'(romacs-agent-send {a.s(waited["taskId"])} "say second turn")')
        until(lambda: sum(1 for e in a.codex_runs() if e['codex'] == run['codex']) >= 2, 30, 'follow-up turn')
        follow = [e for e in a.codex_runs() if e['codex'] == run['codex']][-1]
        a.idle(waited['taskId'])
        cancelled = delegate_call({'task': 'slow 30', 'target': {'provider': 'codex'}, 'clientRequestId': 'c1'})
        until(lambda: any(e['prompt'] == 'slow 30' for e in a.codex_runs()), 20, 'slow child running')
        slow_pid = next(e['pid'] for e in a.codex_runs() if e['prompt'] == 'slow 30')
        a.send(parent, 'call romacs_task_cancel ' + json.dumps({'taskId': cancelled['taskId']}))
        a.idle(parent)
        cancel = json.loads(a.tool_results()[-1][1])
        until(lambda: not Path(f'/proc/{slow_pid}').exists(), 15, 'cancelled child ended')
        # An interrupt ends the run, journaled: the turn failed, the session
        # is idle (not detached), and the next message continues its thread.
        stopped = delegate_call({'task': 'slow 30', 'target': {'provider': 'codex'}, 'clientRequestId': 'i1'})
        until(lambda: sum(1 for e in a.codex_runs() if e['prompt'] == 'slow 30') >= 2, 20, 'second slow child running')
        until(lambda: a.eval(f'(and (process-live-p (romacs-agent-connection {a.s(stopped["taskId"])})) t)') == 't', 20, 'attached')
        a.eval(f'(romacs-agent-interrupt {a.s(stopped["taskId"])})')
        until(lambda: a.state(stopped['taskId']) == 'idle', 15, 'interrupted child idle')
        interrupted = a.eval(f'(romacs-agent-tools--status {a.s(stopped["taskId"])})')
        a.eval(f'(romacs-agent-send {a.s(stopped["taskId"])} "say after interrupt")')
        until(lambda: any(e['prompt'] == 'say after interrupt' for e in a.codex_runs()), 20, 'turn after interrupt')
        a.idle(stopped['taskId'])
        # A wait that ends first (timeoutMs) leaves the child's end to wake the parent.
        expired = delegate_call({'task': 'slow 2', 'mode': 'wait', 'timeoutMs': 500, 'target': {'provider': 'codex'},
                                 'clientRequestId': 't1'})
        until(lambda: any(expired['taskId'] in e.get('wake', '') for e in a.logged()), 30, 'expired wait woken')
        # A child that ends while no server runs: the next server wakes its parent.
        missed = delegate_call({'task': 'slow 3', 'target': {'provider': 'codex'}, 'clientRequestId': 'm1'})
        a.client('--stop=none')
        until(lambda: not Path(f'/proc/{a.pid}').exists(), 15, 'server gone')
        until(lambda: any(e['prompt'] == 'slow 3' for e in a.codex_runs()), 20, 'missed child ran')
        time.sleep(4)
        a.restart()
        until(lambda: any(missed['taskId'] in e.get('wake', '') for e in a.logged()), 30, 'missed wake delivered')
        time.sleep(1)
        cancelled_after = a.eval(f'(romacs-agent-tools--status {a.s(cancelled["taskId"])})')
        wakes = [e['wake'] for e in a.logged() if 'wake' in e]
        result = {'waited': {k: waited.get(k) for k in ('status', 'result', 'provider', 'model', 'role')},
                  'codex_home': run['codex_home'], 'argv': run['argv'],
                  'same_request_same_task': again['taskId'] == waited['taskId'],
                  'async_status': asynced['status'], 'wake': wake[:200],
                  'follow_up_resumed': 'resume' in follow['argv'] and follow['prompt'] == 'say second turn',
                  'cancel': cancel.get('status'), 'cancelled_after_restart': cancelled_after,
                  'interrupted': interrupted, 'expired_wait': expired.get('waitTimedOut'), 'wakes': len(wakes),
                  'cancelled_woke': any(cancelled['taskId'] in w for w in wakes)}
        result['ok'] = (waited['status'] == 'completed' and waited['result'] == 'reviewed'
                        and run['codex_home'] == str(home) and '-m' in run['argv'] and 'm-test' in run['argv']
                        and 'model_reasoning_effort="low"' in run['argv']
                        and result['same_request_same_task'] and asynced['status'] == 'running'
                        and '[completed]' in wake and 'slow done' in wake and result['follow_up_resumed']
                        and cancel.get('status') == 'cancelled' and not result['cancelled_woke']
                        and cancelled_after == '"cancelled"' and interrupted == '"failed"' and expired.get('waitTimedOut')
                        # A wake per child turn end no wait answered, each with its own result: the
                        # async child, the waited child's follow-up, the expired wait, the missed one
                        # (a replay repeats none), and the interrupted child's failed run and next turn.
                        and woken_ids(wakes, [asynced, waited, expired, missed], stopped))
        return result
    finally:
        a.close()


def woken_ids(wakes, once, interrupted):
    ids = [w.split()[2] for w in wakes]
    told = [w for w in wakes if w.split()[2] == interrupted['taskId']]
    return (all(ids.count(task['taskId']) == 1 for task in once) and len(ids) == len(once) + 2
            and len(told) == 2 and '[failed]' in told[0] and told[1].endswith('[completed]:\nafter interrupt'))


def build_dir(a, commit):
    """A build directory for this binary: romacs-vm-gui and its runtime, and
    a build.json naming COMMIT, as an install makes."""
    d = a.root / f'build-{commit[:8]}'
    d.mkdir()
    binary = Path(a.args.romacs).resolve()
    runtime = Path(os.environ.get('ROMACS_RUNTIME_ROOT') or Path(__file__).resolve().parents[3])
    (d / 'romacs-vm-gui').symlink_to(binary)
    for name in ('romacs-mcp', 'romacs-agent-host'):
        (d / name).symlink_to(binary.with_name(name))
    for root in ('gnu-lisp', 'rlisp', 'etc'):
        (d / root).symlink_to(runtime / root)
    (d / 'build.json').write_text(json.dumps({'commit': commit}))
    return d


def restart_unit(args, base):
    """`restart' with the user manager: the server is the unit
    romacs-server-S, and its successor comes from a helper unit once that
    unit has ended."""
    bus = os.environ.get('DBUS_SESSION_BUS_ADDRESS')
    if not bus:
        return {'ok': None, 'skipped': 'no user bus'}
    return restart(args, base, bus)


def restart_race(args, base):
    """`restart' with a launcher racing the successor: a client that starts
    the server when none listens keeps asking during the restart; whichever
    server comes up restores the session, and the requester is told."""
    return restart(args, base, race=True)


def restart(args, base, bus=None, race=False):
    """An agent asks for a restart into a build: it runs once the agent's turn
    ends; the server comes back as the same server (no user manager here:
    the detached path) with the session's buffers; the requesting Claude
    session and an idle Codex session are told "Romacs restarted into
    <commit>" (the Codex one by a new turn)."""
    name = f'ru{os.getpid()}' if bus else 'rs'
    a = Agents(args, base, name, bus and {'DBUS_SESSION_BUS_ADDRESS': bus, 'ROMACS_SERVER_SLICE': 'romacs-build.slice'})
    if bus:
        a.env['WAYLAND_DISPLAY'] = str(a.runtime / a.env['WAYLAND_DISPLAY'])
        a.env['XDG_RUNTIME_DIR'] = os.environ.get('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}')
        a.unit = f'romacs-server-{name}'
    a.persist()
    try:
        a.up()
        in_unit = a.eval('(and (getenv "INVOCATION_ID") t)')
        a.codex_instance()
        target = build_dir(a, 'abcdef0123456789')
        a.eval('(with-current-buffer (get-buffer-create "notes") (erase-buffer) (insert "carried across\\n") t)')
        requester = a.start('say ready')
        a.idle(requester)
        idle_codex = a.start('say codex ready', 'codex')
        a.idle(idle_codex)
        old_pid, harness = a.pid, a.harness_pid(requester)
        # The Codex session is told too, as if it had asked.
        (a.home / '.local/state/romacs/agents' / idle_codex / 'restart-wake').write_text('')
        a.send(requester, 'call romacs_restart_request ' + json.dumps({'target': str(target), 'delay': 1}))
        asked = None
        until(lambda: any(e.get('tool') == 'romacs_restart_request' for e in a.logged()), 30, 'requested')
        asked = next(e for e in a.logged() if e.get('tool') == 'romacs_restart_request')

        def new_pid():
            # Never starting a server (unless racing): the successor must be the one to.
            seconds, out = a.client(*(() if race else ('--no-start',)), '-e', '(emacs-pid)', check=False)
            return int(out) if out.isdigit() else None
        # Asked rarely: a call in flight is in the restart's way.
        time.sleep(1)
        pending = a.eval('(prin1-to-string (list (romacs-agent-state (romacs-agent-get %s)) warm-restart-last-result))'
                         % lisp_text(requester))
        for _ in range(45 if not race else 300):
            time.sleep(2 if not race else 0.3)
            if (p := new_pid()) and p != old_pid:
                break
        else:
            raise RuntimeError(f'not restarted: {pending}')
        a.pid = new_pid()
        until(lambda: any('Romacs restarted' in e.get('wake', '') for e in a.logged()), 30, 'requester told')
        told = next(e['wake'] for e in a.logged() if 'Romacs restarted' in e.get('wake', ''))
        until(lambda: any(e.get('prompt', '').startswith('Romacs restarted') for e in a.codex_runs()), 30, 'codex told')
        codex_told = next(e['prompt'] for e in a.codex_runs() if e.get('prompt', '').startswith('Romacs restarted'))
        carried = a.eval('(and (get-buffer "notes") (with-current-buffer "notes" (buffer-string)))')
        runtime = a.eval('(getenv "ROMACS_RUNTIME_ROOT")')
        served = a.eval('romacs-server-name')
        wake_files = [p for p in (a.home / '.local/state/romacs/agents').glob('*/restart-wake')]
        result = {'in_unit': in_unit, 'unit_after': bus and subprocess.run(
                      ['systemctl', '--user', 'is-active', a.unit], capture_output=True, text=True).stdout.strip(),
                  'asked': asked['text'][:200], 'old_pid': old_pid, 'new_pid': a.pid, 'harness_same': a.harness_pid(requester) == harness,
                  'told': told, 'codex_told': codex_told, 'carried': carried, 'runtime': runtime, 'served': served,
                  'wake_files_left': len(wake_files)}
        # Racing, the launcher's server (this build) may win: it restores the
        # session in its own build, and says which.
        into = 'Romacs restarted into ' if race else 'Romacs restarted into abcdef01'
        result['ok'] = ('"scheduled"' in asked['text'] and a.pid != old_pid and result['harness_same']
                        and told.startswith(into) and codex_told.startswith(into)
                        and carried == '"carried across\\n"' and (race or str(target) in runtime) and served == f'"{name}"'
                        and not wake_files and (in_unit == 't') == bool(bus) and (not bus or result['unit_after'] == 'active'))
        return result
    finally:
        if bus:
            subprocess.run(['systemctl', '--user', 'stop', a.unit], capture_output=True)
        a.close()


GNOSIS_FIXTURE = r"""(progn
  (require 'gnosis)
  (setq gnosis-dir (file-name-as-directory %s) gnosis-testing t gnosis-vc-auto-push nil)
  (make-directory gnosis-dir t)
  (setq gnosis-db (gnosis-sqlite-open (expand-file-name "gnosis.db" gnosis-dir)))
  (gnosis-db-init)
  (load %s nil t)
  (prin1-to-string (list (gnosis-test--add-basic-thema "Capital of France?" "Paris")
                         (gnosis-test--add-basic-thema "Capital of Greece?" "Athens")
                         (gnosis-test--add-basic-thema "Capital of Italy?" "Rome"))))"""


def gnosis(args, base):
    """Gnosis's agent layer through romacs-agent, on a fixture database in the
    private tree: the API's tools through the harness MCP client (paging by
    cursor token, preview and apply with reordered keys, practice reserved and
    cancelled with no grade written); the evaluation adapter through a fake
    Claude and a fake Codex session (pass resolved, malformed rejected), the
    tutor adapter through a fake Claude."""
    a = Agents(args, base, 'gn')
    try:
        a.up()
        a.codex_instance()
        helpers = Path(__file__).resolve().parents[3] / 'test/lisp/gnosis/gnosis-test-helpers.el'
        ids = parity_live.read_sexp(parity_live.read_sexp(a.eval(GNOSIS_FIXTURE % (lisp(a.home / 'gnosis'), lisp(helpers)))))
        with RomacsMcp.for_server(a, agent_tools=True) as mcp:
            offered = [t for t in mcp.tools() if t.startswith('gnosis_')]
            page1 = mcp.call('gnosis_content_search', limit=2)
            page2 = mcp.call('gnosis_content_search', limit=2, cursor=page1['next'])
            records = mcp.call('gnosis_content_fetch', ids=[str(i) for i in ids])
            first = records['items'][0]
            # Keys in another order than fetch gave them.
            change = {'add-tags': ['agent'], 'revision': first['revision'], 'id': first['id']}
            plan = mcp.call('gnosis_content_preview', owner=records['owner'], changes=[change])
            applied = mcp.call('gnosis_content_apply', **{'plan-token': plan['plan-token']})
            practice = mcp.call('gnosis_start_practice', **{'thema-ids': ids[:2], 'limit': 2,
                                                            'policy': {'consecutive': False, 'successes-required': 1}})
            status = mcp.call('gnosis_status', **{'session-id': practice['session-id']})
            # Native answering has the batch now: the API refuses to change it.
            _, refusal, refused = mcp.tool('gnosis_cancel', {'session-id': practice['session-id']})
            sessions = mcp.call('gnosis_sessions', limit=5)
        # Nothing graded: no review event, no practice encounter.
        reviews = a.eval('(+ (caar (sqlite-select gnosis-db "SELECT count(*) FROM review_events")) '
                         '(caar (sqlite-select gnosis-db "SELECT count(*) FROM practice_encounters")))')
        # Asked as a command would: the answer arrives later, from the session.
        ask_form = r"""(progn (defvar gnosis-live--answer nil) (setq gnosis-live--answer nil)
  (funcall %s (list :question "Capital of France?" :reference-answer "Paris" :rubric "Names Paris" :response %s)
           (lambda (r) (setq gnosis-live--answer (list 'resolved r))) (lambda (e) (setq gnosis-live--answer (list 'rejected e))))
  t)"""

        def ask(function, response):
            a.eval(ask_form % (function, response))
            until(lambda: a.eval('gnosis-live--answer') != 'nil', 40, 'evaluation answered')
            return parity_live.read_sexp(a.eval('(prin1-to-string gnosis-live--answer)'))
        a.eval("(progn (require 'gnosis-agent-romacs) t)")
        claude = ask("#'gnosis-agent-eval-romacs", '"Paris"')
        malformed = ask("#'gnosis-agent-eval-romacs", '"MALFORMED"')
        a.eval('(setq gnosis-agent-romacs-provider "codex" gnosis-agent-romacs-model "m-luna")')
        codex = ask("#'gnosis-agent-eval-romacs", '"Paris"')
        codex_run = [e for e in a.codex_runs() if e.get('prompt', '').startswith('Evaluate')][-1]
        a.eval('(setq gnosis-agent-romacs-provider nil gnosis-agent-romacs-model nil)')
        a.eval(r"""(progn (setq gnosis-live--answer nil)
  (gnosis-agent-review-romacs (list :api-version 1 :phase "evaluate" :question (list :id "q1" :question "Capital?"
                                     :reference-answer "Paris" :rubric "Paris" :hints [] :parathema nil :tags [] :source "atlas")
                                    :response "Paris")
                              (lambda (r) (setq gnosis-live--answer (list 'resolved r)))
                              (lambda (e) (setq gnosis-live--answer (list 'rejected e))))
  t)""")
        until(lambda: a.eval('gnosis-live--answer') != 'nil', 40, 'tutor answered')
        tutor = parity_live.read_sexp(a.eval('(prin1-to-string gnosis-live--answer)'))
        asks = parity_live.read_sexp(a.eval(
            "(prin1-to-string (mapcar (lambda (s) (symbol-name (romacs-agent-state s))) "
            "(seq-filter (lambda (s) (equal (plist-get (romacs-agent-options s) :role) \"ask\")) (romacs-agent-sessions))))"))
        result = {'offered': len(offered), 'paged': [len(page1['items']), len(page2['items']), page2['complete']],
                  'applied_tags': applied['items'][0].get('tags'), 'practice': practice.get('status'),
                  'status': status.get('status'), 'cancel_refused': [refused, refusal[:120]], 'sessions': len(sessions['items']),
                  'review_rows': reviews, 'claude': claude, 'malformed': malformed, 'codex': codex,
                  'codex_argv_model': 'm-luna' in codex_run['argv'], 'tutor': tutor, 'ask_states': asks}
        result['ok'] = (len(offered) == 20 and result['paged'] == [2, 1, True] and 'agent' in (result['applied_tags'] or [])
                        and practice.get('status') in ('pending', 'running') and refused and 'native review' in refusal
                        and reviews == '0' and 'resolved' in claude and 'pass' in claude
                        and 'rejected' in malformed and 'no grade recorded' in malformed
                        and 'resolved' in codex and result['codex_argv_model'] and 'resolved' in tutor
                        and all(state in ('ended', 'idle', 'detached') for state in parity_live.read_sexp(asks)))
        return result
    finally:
        a.close()


def launcher(args, base):
    """Editors started as the launcher starts them (no `--server'),
    restarted with their sessions detached.  A session's relay names its
    own route, a link the process attached to it leads to itself: the next
    editor, restoring at startup with no command, attaches and leads each
    route to itself, so a harness's next romacs_eval reaches it.  Two
    editors attached to different sessions each get their own sessions'
    calls; an attach the supervisor refuses changes no route.  A session an
    earlier build started (its relay naming that editor's session-PID path)
    is led the same way.  Stale endpoints of ended processes are removed;
    nothing live or not ours is touched."""
    import socket as sockets
    a = Agents(args, base, 'ln')
    editors = []
    try:
        runtime = a.runtime / 'romacs'
        relay = Path(args.romacs).with_name('romacs-mcp')
        later = '"later 20 call romacs_eval {\\"instance\\": \\"?\\", \\"code\\": \\"(emacs-pid)\\"}"'
        own = lambda editor: runtime / f'agents/editor/{editor.pid}.sock'

        def start():
            log = (a.root / f'editor-{len(editors)}.log').open('w')
            # As the launcher starts it: GNU's Lisp loaded, no server.
            editor = subprocess.Popen([args.romacs, '--loadup'], env=a.env, cwd=a.root / 'work', stdout=log, stderr=log,
                                      stdin=subprocess.DEVNULL)
            editors.append(editor)
            a.pid = editor.pid
            return editor

        def ask(socket, code):
            with RomacsMcp.connect(socket, relay, a.env) as client:
                return client.call('romacs_eval', code=code)

        def stop(editor):
            editor.terminate()
            editor.wait(30)

        def pids(sid):
            return {e.get('pid') for e in a.logged() if e.get('started') == sid}

        def reached(sid, editor, after=0):
            return any(e.get('tool') == 'romacs_eval' and e['text'].strip() == str(editor.pid)
                       and e.get('pid') in pids(sid) and e['t'] > after for e in a.logged())

        def route(sid):
            path = json.loads((a.home / '.local/state/romacs/agents' / sid / 'mcp.json').read_text())
            return Path(path['mcpServers']['romacs']['args'][1])

        def started(socket):
            sid = str(ask(socket, f'(romacs-agent-id (romacs-agent-start {lisp(a.work)} "say ready"))')).strip('"')
            until(lambda: ask(socket, f'(romacs-agent-state (romacs-agent-get {lisp_text(sid)}))') == 'idle', 30, f'{sid} idle')
            return sid

        a.work.mkdir(parents=True, exist_ok=True)
        # Editor 0 is as editors were: its endpoint on its own pid's path,
        # which its sessions' relays name.
        (a.home / '.config/romacs/init.el').write_text(
            (a.home / '.config/romacs/init.el').read_text() + f"""
(setq romacs-agent-claude-program {lisp(HARNESS)})
(when (file-exists-p {lisp(a.root / 'old-endpoint')})
  (run-at-time 0.5 nil
               (lambda ()
                 (require 'romacs-agent)
                 (let ((socket (format "%sromacs/session-%d/server.sock" (file-name-as-directory (getenv "XDG_RUNTIME_DIR")) (emacs-pid))))
                   (make-directory (file-name-directory socket) t)
                   (set-file-modes (file-name-directory socket) #o700)
                   (romacs-mcp-start socket)
                   (defun romacs-agent--route (_) socket)))))
""")
        (a.root / 'old-endpoint').write_text('')
        old = start()
        old_socket = runtime / f'session-{old.pid}/server.sock'
        until(lambda: old_socket.is_socket(), 60, 'the old per-process endpoint')
        pinned = started(old_socket)
        (a.root / 'old-endpoint').unlink()
        # Earlier builds' directories: one of an editor long gone, its socket
        # stale (removed); a live listener under a dead number (a server so
        # named) and a link to a directory elsewhere (both untouched).
        def plant(name, listen=False):
            directory = runtime / name
            directory.mkdir(mode=0o700)
            node = sockets.socket(sockets.AF_UNIX)
            node.bind(str(directory / 'server.sock'))
            if listen:
                node.listen()
                return node
            node.close()
        plant('session-999999')
        listener = plant('session-999998', listen=True)
        elsewhere = a.root / 'elsewhere'
        elsewhere.mkdir()
        (elsewhere / 'server.sock').write_text('not a socket')
        (runtime / 'session-999997').symlink_to(elsewhere)
        ask(old_socket, f'(romacs-agent-send (romacs-agent-get {lisp_text(pinned)}) {later})')
        stop(old)
        # Editor 1: started with no command; it attaches the pinned session
        # and leads that session's old path to itself.
        one = start()
        until(lambda: reached(pinned, one), 60, 'the pinned session reached editor 1')
        linked = os.readlink(old_socket) if old_socket.is_symlink() else None
        fresh = started(own(one))
        fresh_route = route(fresh)
        for sid in (pinned, fresh):
            ask(own(one), f'(romacs-agent-send (romacs-agent-get {lisp_text(sid)}) {later})')
        stop(one)
        two = start()
        until(lambda: reached(pinned, two) and reached(fresh, two), 60, 'both sessions reached editor 2')
        # Editor 3 starts while editor 2 is attached: the supervisors refuse
        # it and no route moves.  Editor 2 then lets the fresh session go
        # (its supervisor connection closed) and editor 3 attaches it: only
        # that session's calls reach editor 3, the pinned one's editor 2.
        three = start()
        until(lambda: own(three).is_socket(), 60, "editor 3's endpoint")
        # Longer than any supervisor takes to refuse (an earlier one, with no
        # acknowledgement, is taken as kept only after 3 s connected).
        time.sleep(4)
        kept = (os.readlink(fresh_route), os.readlink(old_socket))
        ask(own(two), f'(progn (delete-process (romacs-agent-connection (romacs-agent-get {lisp_text(fresh)}))) t)')
        ask(own(three), f'(progn (romacs-agent--attach (romacs-agent-get {lisp_text(fresh)}) 10) t)')
        until(lambda: os.readlink(fresh_route) == str(own(three)), 15, "the fresh route led to editor 3")
        mark = time.time()
        ask(own(three), f'(romacs-agent-send (romacs-agent-get {lisp_text(fresh)}) {later})')
        ask(own(two), f'(romacs-agent-send (romacs-agent-get {lisp_text(pinned)}) {later})')
        until(lambda: reached(fresh, three, mark) and reached(pinned, two, mark), 60, 'each session reached its own editor')
        # A supervisor that refuses late (another server's long replay held
        # it): an attach it never keeps moves no route, even with the
        # endpoint replaced (a promotion) while the refusal is pending.
        import threading
        late_id = '22222222-3333-4444-8555-666666666666'
        late_dir = a.home / '.local/state/romacs/agents' / late_id
        late_dir.mkdir(mode=0o700)
        late_socket = runtime / 'agents/late.sock'
        late = sockets.socket(sockets.AF_UNIX)
        late.bind(str(late_socket))
        late.listen()

        def refuse_late():
            peer, _ = late.accept()
            peer.recv(4096)
            time.sleep(6)
            peer.sendall(b'{"refused": "attached", "pid": 1}\n')
            peer.close()
        threading.Thread(target=refuse_late, daemon=True).start()
        (late_dir / 'manifest.json').write_text(json.dumps(
            {'id': late_id, 'harness': 'claude', 'provider': 'claude', 'configDir': None, 'cwd': str(a.work),
             'title': 'late', 'hostSocket': str(late_socket), 'argv': [], 'options': {}}))
        late_route = runtime / f'agents/route/{late_id}.sock'
        late_started = time.time()
        ask(own(three), f'(progn (romacs-agent--load {lisp(late_dir)}) t)')
        # Editor 3 becomes a named server: its endpoint is replaced, and the
        # session attached to it leads to the new one.
        try:
            # Answered by nothing: the call's own endpoint is the one replaced.
            ask(own(three), '(progn (run-at-time 0 nil (lambda () (romacs-server-start "promoted"))) t)')
        except Exception:  # noqa: BLE001
            pass
        promoted = runtime / 'promoted/server.sock'
        until(lambda: os.readlink(fresh_route) == str(promoted), 15, 'the fresh route led to the promoted server')
        mark = time.time()
        ask(promoted, f'(romacs-agent-send (romacs-agent-get {lisp_text(fresh)}) {later})')
        until(lambda: reached(fresh, three, mark), 60, 'the fresh session reached the promoted server')
        time.sleep(max(0, late_started + 7 - time.time()))
        late_moved = os.path.lexists(late_route)
        late.close()
        # Both gone, a named server starts: it attaches both, leads both routes.
        for sid, socket in ((fresh, promoted), (pinned, own(two))):
            ask(socket, f'(romacs-agent-send (romacs-agent-get {lisp_text(sid)}) {later})')
        stop(two)
        stop(three)
        mark = time.time()
        a.up()
        until(lambda: reached(fresh, a, mark - 25) and reached(pinned, a, mark - 25), 60, 'both sessions reached the server')
        listener.close()
        left = sorted(p.name for p in runtime.glob('session-*'))
        editor_dir = sorted(p.name for p in (runtime / 'agents/editor').iterdir())
        result = {'editors': [e.pid for e in editors], 'fresh_route': str(fresh_route), 'old_link': linked,
                  'kept_on_refusal': kept, 'late_refusal_moved_route': late_moved,
                  'session_dirs_left': left, 'editor_dir_at_end': editor_dir,
                  'evals': [e['text'] for e in a.logged() if e.get('tool') == 'romacs_eval']}
        result['ok'] = (fresh_route.parent == runtime / 'agents/route' and linked == str(own(one))
                        and kept == (str(own(two)), str(own(two))) and not late_moved
                        and left == sorted([f'session-{old.pid}', 'session-999997', 'session-999998'])
                        and (elsewhere / 'server.sock').read_text() == 'not a socket'
                        and editor_dir == [])
        return result
    finally:
        for editor in editors:
            if editor.poll() is None:
                editor.kill()
        a.close()


UNICODE = 'it’s café 中文 😀 é ok'


def unicode(args, base):
    """Non-ASCII through every boundary of a session: the prompt (’, é,
    CJK, emoji, a combining mark) to the harness and its reply, the
    journal, the chat and its title, the manifest after a restart, a tool's
    input and output, a buffer read with a raw byte in it (its byte written
    out, the rest intact, not the whole result refused), a Codex turn, and a
    harness transcript imported from history."""
    a = Agents(args, base, 'uc')
    try:
        a.up()
        a.work.mkdir(parents=True, exist_ok=True)
        sid = a.start(f'say {UNICODE}')
        a.idle(sid)
        chat = a.chat(sid)
        journal = a.journal(sid)
        sent = next((r['in'] for r in journal if isinstance(r.get('in'), dict)), {})
        sent_text = (sent.get('message') or {}).get('content')
        results = a.results(sid)
        title = a.eval(f'(romacs-agent-title {a.s(sid)})')
        buffer_name = a.eval(f'(buffer-name (romacs-agent-buffer {a.s(sid)}))')
        # A tool's input and output.
        a.send(sid, 'call romacs_eval ' + json.dumps({'instance': '?', 'code': f'(concat "{UNICODE}")'}))
        a.idle(sid)
        evaluated = a.tool_results()[-1]
        # A raw byte in a buffer a tool reads.
        a.eval('(with-current-buffer (get-buffer-create "raw") (erase-buffer) '
               f'(insert "a" (unibyte-string 255) "b {UNICODE}") t)')
        a.send(sid, 'call romacs_buffer_read ' + json.dumps({'instance': '?', 'name': 'raw', 'start': 1, 'maxChars': 100}))
        a.idle(sid)
        read = a.tool_results()[-1]
        chat_after = a.chat(sid)
        # A session in a non-ASCII directory, the editor's locale Latin-1:
        # the harness starts there all the same.
        odd = a.work / 'caf\u00e9-w\u00f6rk'
        odd.mkdir()
        other = a.eval(f'(let ((locale-coding-system (quote latin-1)) (default-process-coding-system (quote (latin-1 . latin-1)))) '
                       f'(romacs-agent-id (romacs-agent-start {lisp_text(str(odd))} "say ok")))').strip('"')
        a.idle(other)
        started_in = next((e.get('cwd') for e in a.logged() if e.get('started') == other), None)
        # A Codex turn.
        a.codex_instance()
        codex = a.start(f'say {UNICODE}', 'codex')
        a.idle(codex)
        codex_reply = a.eval(f'(romacs-agent-reply {a.s(codex)})')
        # The manifest's title after a restart.
        a.client('--stop=none')
        until(lambda: not Path(f'/proc/{a.pid}').exists(), 15, 'server gone')
        a.restart()
        title_again = a.eval(f'(romacs-agent-title {a.s(sid)})')
        # A transcript in the harness's history with non-ASCII.
        claude_dir = a.home / '.config/claude/acct'
        cid = '33333333-4444-4555-8666-777777777777'
        project = claude_dir / 'projects' / str(a.work).replace('/', '-')
        project.mkdir(parents=True)
        (project / f'{cid}.jsonl').write_text(json.dumps(
            {'type': 'user', 'isSidechain': False, 'cwd': str(a.work), 'timestamp': '2026-11-04T10:00:00Z',
             'message': {'role': 'user', 'content': UNICODE}}, ensure_ascii=False, separators=(',', ':')) + '\n',
            encoding='utf-8')
        a.providers([('claude', claude_dir)])
        listed = parity_live.read_sexp(a.eval('(progn (require (quote romacs-agent-history)) (romacs-agent-history-list) '
                                              '(mapcar (lambda (i) (plist-get i :title)) (romacs-agent-history-items)))')) or []
        view = parity_live.read_sexp(a.eval(f'(with-current-buffer (save-window-excursion (romacs-agent-history-open '
                                            f'(seq-find (lambda (i) (equal (plist-get i :id) {lisp_text(cid)})) (romacs-agent-history-items)))) '
                                            '(buffer-substring-no-properties (point-min) (point-max)))'))
        result = {'chat': chat[:300], 'sent': sent_text, 'results': results, 'title': title, 'buffer': buffer_name,
                  'evaluated': evaluated, 'read': read, 'codex_reply': codex_reply, 'title_again': title_again,
                  'listed': listed, 'view': (view or '')[:200], 'started_in': started_in}
        result['ok'] = (f'> say {UNICODE}' in chat and UNICODE in chat.split(f'> say {UNICODE}', 1)[1]
                        and sent_text == f'say {UNICODE}' and results and results[0] == UNICODE
                        and UNICODE in title and UNICODE in buffer_name
                        and UNICODE in evaluated[1] and not evaluated[2]
                        and json.loads(read[1])['text'] == 'a\ufffdb ' + UNICODE and not read[2]
                        and started_in == str(odd)
                        and UNICODE in chat_after.split('romacs_eval', 1)[1]
                        and UNICODE in codex_reply and UNICODE in title_again
                        and any(UNICODE in t for t in listed) and f'> {UNICODE}' in (view or ''))
        return result
    finally:
        a.close()


def environment(args, base):
    """A harness gets the editor's environment but for what no harness
    inherits: the editor's own launch variables (its build, its usage log,
    its font), an orchestrator's (T3 Code's), and every harness's own (a
    parent Claude Code's or Codex's session, their configuration
    variables).  Only the session's own configuration variable is set,
    for its instance.  So under both launches: a user unit (which also
    unsets them from the user manager's environment) and the detached
    fallback (systemd-run failing)."""
    a = Agents(args, base, 'ev')
    try:
        a.up()
        launch = {'ROMACS_RUNTIME_ROOT': '/installed/build', 'ROMACS_LISP_DIR': '/installed/build/gnu-lisp',
                  'ROMACS_USAGE_LOG': '/sessions/ended/usage.jsonl', 'ROMACS_GUI_FONT': 'Iosevka',
                  'ROMACS_GUI_FONT_SIZE': '15', 'ROMACS_AGENT_TEST_KEPT': 'kept'}
        parent = {'CLAUDECODE': '1', 'CLAUDE_CODE_SESSION_ID': 'parent', 'CLAUDE_CONFIG_DIR': '/parent/claude',
                  'CLAUDE_CODE_ENTRYPOINT': 'sdk-ts', 'T3CODE_THREAD_ID': 'thread-1', 'T3CODE_CLI_PATH': '/t3',
                  'CODEX_HOME': '/parent/codex', 'CODEX_THREAD_ID': 'parent-thread', 'CODEX_CI': '1'}
        dropped = [k for k in {**launch, **parent} if k != 'ROMACS_AGENT_TEST_KEPT']
        a.eval('(progn ' + ' '.join(f'(setenv {lisp_text(k)} {lisp_text(v)})' for k, v in {**launch, **parent}.items()) + ' t)')
        codex_home = a.codex_instance()
        a.work.mkdir(parents=True, exist_ok=True)

        def children(tag):
            """A Claude session (its default directory: no variable) and a
            Codex one (its own home), their logged environments."""
            sid = a.start(f'say ready {tag}')
            a.idle(sid)
            tid = a.start(f'say ready {tag}', 'codex')
            until(lambda: any(e.get('prompt') == f'say ready {tag}' for e in a.codex_runs()), 30, f'codex {tag} ran')
            claude = next(e.get('env', {}) for e in a.logged() if e.get('started') == sid)
            codex = next(e.get('env', {}) for e in a.codex_runs() if e.get('prompt') == f'say ready {tag}')
            return sid, tid, claude, codex

        sid, tid, claude, codex = children('unit')
        # A command the agent's own evaluation starts in the editor.
        a.send(sid, 'call romacs_eval ' + json.dumps(
            {'instance': '?', 'code': '(shell-command-to-string "env | grep -E \'^(ROMACS_|CLAUDE|CODEX_|T3CODE_)\' | sort")'}))
        a.idle(sid)
        from_eval = a.tool_results()[-1][1]
        # The unit unsets them even from the user manager's environment.
        unit = a.eval(f'(romacs-agent-unit {a.s(sid)})').strip('"')
        unset = subprocess.run(['systemctl', '--user', 'show', '-p', 'UnsetEnvironment', unit],
                               capture_output=True, text=True).stdout.strip() if unit != 'nil' else 'no unit'
        # The detached fallback: a systemd-run that fails.
        stub = a.root / 'no-systemd'
        stub.mkdir()
        (stub / 'systemd-run').write_text('#!/bin/sh\nexit 1\n')
        (stub / 'systemd-run').chmod(0o755)
        a.eval(f'(progn (setq exec-path (cons {lisp(stub)} exec-path)) t)')
        sid2, tid2, claude2, codex2 = children('detached')
        detached = [a.eval(f'(romacs-agent-unit {a.s(s)})') for s in (sid2,)]

        def clean(env, own=None):
            return (not any(k in env for k in dropped if k != own)
                    and env.get('ROMACS_AGENT_TEST_KEPT') == 'kept')
        result = {'claude_env': claude, 'codex_env': codex, 'claude_detached': claude2, 'codex_detached': codex2,
                  'eval_env': from_eval, 'unit_unset': unset, 'detached_units': detached}
        result['ok'] = (clean(claude) and clean(claude2)
                        and clean(codex, 'CODEX_HOME') and codex.get('CODEX_HOME') == str(codex_home)
                        and clean(codex2, 'CODEX_HOME') and codex2.get('CODEX_HOME') == str(codex_home)
                        and not any(k in from_eval for k in dropped) and 'ROMACS_AGENT_TEST_KEPT' in from_eval
                        and (unset == 'no unit' or all(k in unset for k in dropped))
                        and detached == ['nil'])
        return result
    finally:
        a.close()


def mcp_tour(args, base):
    """Every tool the endpoint offers, called once through the MCP client a
    harness uses (romacs_mcp_client, the romacs-mcp relay), each answering
    with a result or a tool error, never a protocol error or silence; the
    server's initialize instructions name romacs_identity, romacs_observe,
    romacs_eval and romacs_help; observe's buffers are counts by kind, and
    names (a private chat's only with private) when asked.  Gnosis's tools,
    with no session or database here, answer their refusals."""
    a = Agents(args, base, 'to')
    try:
        a.up()
        a.work.mkdir(parents=True, exist_ok=True)
        with RomacsMcp.for_server(a, agent_tools=True) as mcp:
            instructions = mcp.server.get('instructions', '')
            names = mcp.tools()
            mcp.call('romacs_eval', code='(progn (with-current-buffer (get-buffer-create "*tour*") (erase-buffer) (insert "abc")) '
                                         '(get-buffer-create "*-jabber-chat-friend@example.org*") t)')
            tick = json.loads(mcp.tool('romacs_buffer_read', {'instance': mcp.identity(), 'name': '*tour*', 'start': 1,
                                                              'maxChars': 10})[1]).get('tick')
            note = a.work / 'note.txt'
            note.write_text('one\n')
            plan = {
                'romacs_identity': {}, 'romacs_help': {}, 'romacs_observe': {},
                'romacs_eval': {'code': '(+ 1 2)'},
                'romacs_buffer_list': {'offset': 0, 'limit': 8}, 'romacs_buffer_read': {'name': '*tour*', 'start': 1, 'maxChars': 10},
                'romacs_buffer_edit': {'name': '*tour*', 'expectedTick': tick, 'edits': [{'start': 1, 'end': 2, 'text': 'X'}]},
                'romacs_visit': {'path': str(note)},
                'romacs_file_write': {'path': str(note), 'text': 'two\n', 'save': True},
                'romacs_send_keys': {'keys': 'C-a'}, 'romacs_wait_idle': {'timeout': 5},
                'romacs_call_outcome': {'id': 'no-such-call'},
                'romacs_option_search': {'query': 'kill-ring'},
                'romacs_option_describe': {'name': 'kill-ring-max'},
                'romacs_option_set': {'name': 'kill-ring-max', 'value': '61'},
                'romacs_option_save': {'name': 'kill-ring-max', 'value': '62'},
                'romacs_orchestrator_capabilities': {},
                'romacs_delegate_task': {'task': 'say toured', 'cwd': str(a.work)},
            }
            answered, errors, protocol, silent = {}, {}, {}, []

            def given(name):
                """PLAN's arguments for NAME, else schema-valid ones: each
                required field a value of its type."""
                schema = mcp.schemas().get(name, {})
                arguments = dict(plan.get(name, {}))
                for field in schema.get('required', []):
                    if field not in arguments:
                        kind = schema.get('properties', {}).get(field, {}).get('type')
                        arguments[field] = {'string': 'tour', 'integer': 1, 'number': 1, 'boolean': False,
                                            'array': [], 'object': {}}.get(kind, 'tour')
                if 'instance' in schema.get('properties', {}):
                    arguments['instance'] = mcp.identity()
                return arguments

            def call(name, arguments):
                """Its answer: a result, a tool error, a protocol error or none."""
                _, reply = mcp.request('tools/call', {'name': name, 'arguments': arguments}, 60)
                if 'error' in reply:
                    (protocol.__setitem__(name, reply['error']) if 'code' in reply['error'] else silent.append(name))
                    return None
                text = ''.join(c.get('text', '') for c in reply['result'].get('content', []))
                answered[name] = text
                if reply['result'].get('isError'):
                    errors[name] = text[:200]
                return text

            for name in [n for n in names if n not in ('romacs_task_status', 'romacs_task_cancel', 'romacs_restart_request')]:
                call(name, given(name))
            task = (json.loads(answered['romacs_delegate_task']).get('taskId')
                    if 'romacs_delegate_task' in answered and 'romacs_delegate_task' not in errors else None)
            for name in ('romacs_task_status', 'romacs_task_cancel'):
                if name in names:
                    call(name, {'instance': mcp.identity(), 'taskId': task or 'none'})
            observed = mcp.call('romacs_observe')
            listed = mcp.call('romacs_observe', buffers=True)
            private = mcp.call('romacs_observe', buffers=True, private=True)
            helped = answered.get('romacs_help', '')
            # Last: it restarts the editor once this caller's turn ends.
            if 'romacs_restart_request' in names:
                call('romacs_restart_request', {'instance': mcp.identity(), 'delay': 3600})
        friend = '*-jabber-chat-friend@example.org*'
        result = {'tools': names, 'errors': errors, 'protocol_errors': protocol, 'no_reply': silent,
                  'instructions': instructions, 'help': helped[:600],
                  'buffers_default': observed.get('buffers'), 'buffers_listed': listed.get('buffers'),
                  'project': observed.get('buffer', {}).get('project'), 'last_user_error': observed.get('lastUserError'),
                  'unanswered': [n for n in names if n not in answered and n not in protocol and n not in silent]}
        result['ok'] = (not result['unanswered'] and not protocol and not silent and len(names) >= 16
                        and all(n in instructions for n in ('romacs_identity', 'romacs_observe', 'romacs_eval', 'romacs_help'))
                        and helped.startswith('core:') and 'agents:' in helped
                        and isinstance(observed.get('buffers'), dict) and 'byKind' in observed['buffers']
                        and friend not in json.dumps(observed['buffers'])
                        and '*tour*' in listed['buffers'] and friend not in listed['buffers']
                        and friend in private['buffers']
                        # Gnosis's refuse without a session or database (none here).
                        and all(n.startswith('gnosis_') or n in ('romacs_call_outcome', 'romacs_restart_request')
                                for n in errors)
                        and task is not None)
        return result
    finally:
        a.close()


def instances(args, base):
    """T3 Code's instances through the `t3' entry, as an init recommends:
`(t3 ("claude" . "~/.claude") ("claude2" . DIR) ("codex" . "~/.codex"))'.
    A fixture of T3's settings (its two defaults implicit, a second Claude
    Code account as T3 stores it, a Codex account overlay) gives claude,
    codex and claude2 from T3 (the Lisp fallbacks unused), the overlay
    disabled with why; the harness's own capabilities call lists them; a
    session started by the T3 id runs that instance's directory."""
    a = Agents(args, base, 'in')
    try:
        second = a.home / '.config/claude/second'
        second.mkdir(parents=True)
        (a.home / '.codex').mkdir()
        t3 = a.home / '.t3/userdata/settings.json'
        t3.parent.mkdir(parents=True)
        t3.write_text(json.dumps({'providerInstances': {
            'claudeAgent_claude2': {'driver': 'claudeAgent', 'displayName': 'Claude2', 'enabled': True,
                                    'config': {'homePath': '~/.config/claude/second'}},
            'codex_personal': {'driver': 'codex', 'displayName': 'Codex Personal',
                               'config': {'shadowHomePath': '~/.codex-t3/personal'}}}}))
        a.up()
        a.eval('(setq romacs-agent-providers (quote (t3 ("claude" . "~/.claude") ("claude2" . "~/.config/claude/other") '
               '("codex" . "~/.codex"))))')
        sid = a.start('say ready')
        a.idle(sid)
        a.send(sid, 'call romacs_orchestrator_capabilities ' + json.dumps({'instance': '?'}))
        a.idle(sid)
        capabilities = json.loads(a.tool_results()[-1][1])
        providers = {p['providerInstanceId']: p for p in capabilities.get('providers', [])}
        # A T3 id starts its instance.
        aliased = a.eval('(romacs-agent-provider (romacs-agent-start default-directory "say hi" nil "claudeAgent_claude2"))').strip('"')
        until(lambda: any(e.get('config_dir') == str(second) for e in a.logged()), 20, 'claude2 started')
        result = {'providers': {k: {f: v.get(f) for f in ('source', 'enabled', 't3InstanceId', 'configDir')}
                                for k, v in providers.items()},
                  'constraints': capabilities.get('constraints'), 'default': capabilities.get('default'), 'aliased': aliased}
        result['ok'] = (list(providers) == ['claude', 'codex', 'claude2', 'codex-personal']
                        and all(providers[n]['source'] == 't3' and providers[n]['enabled'] for n in ('claude', 'codex', 'claude2'))
                        and providers['claude2']['configDir'] == str(second)
                        and providers['claude2']['t3InstanceId'] == 'claudeAgent_claude2'
                        and providers['codex-personal']['enabled'] is False
                        and any('T3 auth overlay' in c['reason'] and c['id'] == 'codex_personal'
                                for c in capabilities.get('constraints', []))
                        and capabilities.get('default') == 'claude' and aliased == 'claude2')
        return result
    finally:
        a.close()


def rpc(args, base):
    """A harness Romacs learns of by its driver and nothing else: fakerpc
    (harness/fakerpc.el, defined by this test) runs fake_rpc.py, which takes
    no prompt before `initialize' then `session/new' (or `session/load'),
    each step after the last one's answer.  Two turns and an approval; a
    server restart while its `initialize' is unanswered: no step sent again,
    a message sent meanwhile held until it is ready; unsolicited lines before
    a step's echo: still one copy; a stop and resume: `initialize' and
    `session/load' again, request ids still rising; its transcripts listed,
    viewed and imported, the same conversation id in two configuration
    directories two sessions."""
    a = Agents(args, base, 'rp')
    try:
        homes = [a.home / '.fakerpc-a', a.home / '.fakerpc-b']
        # Compact, as harnesses write their transcripts.
        line = lambda o: json.dumps(o, separators=(',', ':')) + '\n'
        for home, title in zip(homes, ('fixture in a', 'fixture in b')):
            (home / 'sessions').mkdir(parents=True)
            (home / 'sessions' / 'rpc-same.jsonl').write_text(
                line({'type': 'prompt', 'text': f'say {title}', 'cwd': str(a.work)}) + line({'type': 'reply', 'text': title}))
        (homes[0] / 'sessions' / 'rpc-fixture.jsonl').write_text(
            line({'type': 'prompt', 'text': 'say from the fixture', 'cwd': str(a.work)})
            + line({'type': 'reply', 'text': 'from the fixture'}))
        a.work.mkdir(parents=True, exist_ok=True)
        # The driver, from this test alone, in the init too: a restarted
        # server restores its sessions knowing it.
        setup = (f"(progn (load {lisp(RPC_DRIVER)}) (setq fakerpc-program (list \"python3\" {lisp(RPC)})) "
                 f"(setq romacs-agent-providers (list (cons \"fakerpc\" {lisp(homes[0])}) "
                 f"(list \"fakerpc2\" :harness 'fakerpc :config-dir {lisp(homes[1])}))) t)")
        (a.home / '.config/romacs').mkdir(parents=True, exist_ok=True)
        with open(a.home / '.config/romacs/init.el', 'a') as init:
            init.write(f"\n(setq romacs-agent-claude-program {lisp(HARNESS)})\n(with-eval-after-load 'romacs-agent {setup})\n")
        a.up()
        a.eval(setup)
        rpcs = lambda: [e for e in a.logged() if 'rpc' in e or 'rpc_started' in e]
        done = lambda sid: int(a.eval(f'(romacs-agent-done-seq {a.s(sid)})'))

        def turn(sid, text, expect):
            before = done(sid)
            a.send(sid, text)
            until(lambda: done(sid) > before, 30, f'{text} answered')
            until(lambda: expect in a.chat(sid), 10, f'{expect} shown')

        # Two turns and an approval.
        sid = a.start('say one', 'fakerpc')
        until(lambda: done(sid) > 0, 30, 'first turn')
        a.send(sid, 'ask ls -la')
        until(lambda: a.state(sid) == 'waiting-approval', 30, 'approval asked')
        a.eval(f'(romacs-agent-respond {a.s(sid)} (car (hash-table-keys (romacs-agent-approvals {a.s(sid)}))) t)')
        until(lambda: 'permission allow' in a.chat(sid), 30, 'approval answered')
        a.idle(sid)
        turn(sid, 'say two', 'two')
        native = a.eval(f'(romacs-agent-harness-session {a.s(sid)})').strip('"')
        run1 = [e for e in rpcs() if e.get('rpc') in ('initialize', 'session/new', 'session/load', 'session/prompt')]
        # Stopped and resumed: its startup again, going on with its session.
        a.eval(f'(romacs-agent-stop {a.s(sid)})')
        until(lambda: a.state(sid) in ('ended', 'detached'), 15, 'stopped')
        a.eval(f'(romacs-agent-resume {a.s(sid)})')
        turn(sid, 'say three', 'three')
        run2 = [e for e in rpcs() if e.get('rpc') in ('initialize', 'session/new', 'session/load', 'session/prompt')][len(run1):]
        manifest = json.loads((a.home / '.local/state/romacs/agents' / sid / 'manifest.json').read_text())

        # Its chat drawn again (the journal replayed into a copy): nothing
        # is written to the harness.
        written = len(rpcs())
        a.eval(f'(progn (kill-buffer (romacs-agent-buffer {a.s(sid)})) (romacs-agent-show {a.s(sid)}) t)')
        time.sleep(1)
        redraw_writes = [e.get('rpc') for e in rpcs()[written:]]

        # Stopped, resumed, and the server restarted while that run's
        # `initialize' is unanswered: the last run's startup is not taken
        # for this one's; a message sent meanwhile waits for its readiness.
        a.eval(f'(romacs-agent-stop {a.s(sid)})')
        until(lambda: a.state(sid) in ('ended', 'detached'), 15, 'stopped again')
        a.eval('(progn (setenv "FAKERPC_SLOW_INIT" "4") t)')
        seen_pids = {e.get('pid') for e in rpcs()}
        a.eval(f'(romacs-agent-resume {a.s(sid)})')
        until(lambda: any(e.get('rpc') == 'initialize' and e.get('pid') not in seen_pids for e in rpcs()), 15, 'initialize read')
        a.client('--stop=none')
        until(lambda: not Path(f'/proc/{a.pid}').exists(), 15, 'server gone')
        a.restart()
        until(lambda: a.eval(f'(and {a.s(sid)} t)') == 't', 15, 'restored')
        held_state = a.eval(f'(progn (romacs-agent-send {a.s(sid)} "say held") '
                            f'(list (romacs-agent-queue-length {a.s(sid)}) (romacs-agent--ready-p {a.s(sid)})))')
        before = done(sid)
        until(lambda: done(sid) > before, 30, 'held message answered')
        slow_pid = next(e['pid'] for e in rpcs() if e.get('rpc') == 'initialize' and e['pid'] not in seen_pids)
        slow_run = [(e.get('rpc'), e.get('session')) for e in rpcs() if e.get('pid') == slow_pid and 'rpc' in e]
        a.eval('(progn (setenv "FAKERPC_SLOW_INIT" nil) t)')

        # A new session's server restart in the middle of its startup.
        a.eval('(progn (setenv "FAKERPC_SLOW_INIT" "4") t)')
        seen_pids = {e.get('pid') for e in rpcs()}
        fresh = a.start('', 'fakerpc')
        until(lambda: any(e.get('rpc') == 'initialize' and e.get('pid') not in seen_pids for e in rpcs()), 15, 'fresh initialize read')
        a.client('--stop=none')
        until(lambda: not Path(f'/proc/{a.pid}').exists(), 15, 'server gone again')
        a.restart()
        until(lambda: a.eval(f'(and {a.s(fresh)} t)') == 't', 15, 'fresh restored')
        a.send(fresh, 'say fresh')
        until(lambda: done(fresh) > 0, 30, 'fresh answered')
        fresh_pid = next(e['pid'] for e in rpcs() if e.get('rpc') == 'initialize' and e['pid'] not in seen_pids)
        fresh_run = [e.get('rpc') for e in rpcs() if e.get('pid') == fresh_pid and 'rpc' in e]
        a.eval('(progn (setenv "FAKERPC_SLOW_INIT" nil) t)')

        # Its history: listed, viewed, imported; one id in two homes.
        items = parity_live.read_sexp(a.eval(
            "(progn (require (quote romacs-agent-history)) (romacs-agent-history-list) (mapcar (lambda (i) (mapconcat (function identity) (list (plist-get i :id) (plist-get i :provider) "
            "(plist-get i :title)) \"|\")) (seq-filter (lambda (i) (eq (plist-get i :harness) 'fakerpc)) (romacs-agent-history-items))))")) or []
        items = [i.split('|') for i in items]
        item = lambda ident, provider: (f"(seq-find (lambda (i) (and (equal (plist-get i :id) {lisp_text(ident)}) "
                                         f"(equal (plist-get i :provider) {lisp_text(provider)}))) (romacs-agent-history-items))")
        view = parity_live.read_sexp(a.eval(
            f'(with-current-buffer (save-window-excursion (romacs-agent-history-open {item("rpc-fixture", "fakerpc")})) '
            '(list buffer-read-only (buffer-substring-no-properties (point-min) (point-max))))'))
        imported = [a.eval(f'(romacs-agent-id (romacs-agent-import (quote fakerpc) "rpc-same" '
                           f'(plist-get {item("rpc-same", p)} :path) {lisp(a.work)} "same" {lisp_text(p)} {lisp(h)}))').strip('"')
                    for p, h in (('fakerpc', homes[0]), ('fakerpc2', homes[1]))]
        until(lambda: sum(1 for e in rpcs() if e.get('rpc') == 'session/load' and e.get('session') == 'rpc-same') >= 2,
              30, 'both imports loaded')
        loads = sorted(e.get('home') for e in rpcs() if e.get('rpc_started')
                       and e.get('pid') in {x['pid'] for x in rpcs() if x.get('rpc') == 'session/load' and x.get('session') == 'rpc-same'})
        turn(imported[1], 'say in b', 'in b')
        ids = [e['id'] for e in run1 + run2]
        methods = lambda run: [e['rpc'] for e in run]
        result = {'run1': methods(run1), 'run2': methods(run2), 'native': native, 'manifest_native': manifest.get('harnessSession'),
                  'held_state': held_state, 'slow_run': slow_run, 'fresh_run': fresh_run,
                  'redraw_writes': redraw_writes, 'items': items, 'view': view, 'imported': imported,
                  'loads': loads, 'ids': ids}
        result['ok'] = (methods(run1) == ['initialize', 'session/new', 'session/prompt', 'session/prompt', 'session/prompt']
                        and methods(run2) == ['initialize', 'session/load', 'session/prompt']
                        and run2[1].get('session') == native and native.startswith('rpc-')
                        and manifest.get('harnessSession') == native
                        and ids == sorted(ids) and len(set(ids)) == len(ids)
                        and redraw_writes == []
                        and [m for m, _ in slow_run] == ['initialize', 'session/load', 'session/prompt']
                        and slow_run[1][1] == native and slow_run[2][1] == native
                        and held_state == '(1 nil)'
                        and fresh_run == ['initialize', 'session/new', 'session/prompt']
                        and sorted(i[1] for i in items if i[0] == 'rpc-same') == ['fakerpc', 'fakerpc2']
                        and ['rpc-fixture', 'fakerpc', 'say from the fixture'] in items
                        and view and view[0] == 't' and 'from the fixture' in view[1]
                        and len(set(imported)) == 2
                        and loads == sorted([str(homes[0]), str(homes[1])])
                        and 'in b' in a.chat(imported[1]))
        return result
    finally:
        a.close()


def logging(args, base):
    """A server whose environment names no usage log keeps its own session
    record, as the launcher keeps a session's: STATE/romacs/sessions/
    START-PID/ with usage.jsonl and session.json.  An error a real key
    causes is logged with the functions its signal went through (first of
    its kind), a command slow in its command loop has its slow record, as
    does one fast itself whose redisplay is slow (the host's time of it), a
    stop writes the record's end, and the unit's end (`--server-exited',
    as systemd runs it for this invocation) fills how it exited in."""
    a = Agents(args, base, 'lg', env_extra={'INVOCATION_ID': 'live-logging-1'})
    try:
        a.up()
        sessions = a.home / '.local/state/romacs/sessions'
        until(lambda: any(sessions.glob('*/session.json')), 20, 'the session record')
        directory = next(sessions.glob('*/session.json')).parent
        a.keyboard_ready()
        time.sleep(1.5)

        # A command that fails and one that is slow, on keys (M-: would
        # enter the debugger, as `eval-expression-debug-on-error' asks).
        a.eval('(progn (defun logging-live-fail () (interactive) (car 1)) '
               '(defun logging-live-slow () (interactive) (sleep-for 0.15)) '
               '(global-set-key (kbd "<f9>") (function logging-live-fail)) '
               '(global-set-key (kbd "<f8>") (function logging-live-slow)) '
               # Fast itself; the redisplay after it slow (its header
               # line's :eval spins 80 ms, once).
               '(defvar logging-live-spin 0) '
               '(defun logging-live-spin () (let ((end (+ (float-time) (/ logging-live-spin 1000.0)))) '
               '(setq logging-live-spin 0) (while (< (float-time) end))) "spun") '
               '(defun logging-live-paint () (interactive) (setq logging-live-spin 80) '
               '(setq header-line-format (list (list :eval (list (quote logging-live-spin)))))) '
               '(global-set-key (kbd "<f7>") (function logging-live-paint)) t)')
        for key in ('F9', 'F9', 'F8', 'F7'):
            a.keys('-k', key)
            time.sleep(1)
        a.eval('(progn (romacs-usage-log-flush) t)')
        lines = [json.loads(line) for line in (directory / 'usage.jsonl').read_text().splitlines()]
        errors = [l for l in lines if l.get('kind') == 'error' and l.get('command') == 'logging-live-fail']
        slow = [l for l in lines if l.get('kind') == 'slow' and l.get('command') == 'logging-live-slow']
        painted = [l for l in lines if l.get('kind') == 'slow' and l.get('command') == 'logging-live-paint']
        started = json.loads((directory / 'session.json').read_text())
        a.client('--stop=none')
        until(lambda: not Path(f'/proc/{a.pid}').exists(), 15, 'server gone')
        own_end = json.loads((directory / 'session.json').read_text())
        subprocess.run([args.romacs, '--server-exited', a.name], check=True, timeout=30,
                       env={**a.env, 'INVOCATION_ID': 'live-logging-1', 'SERVICE_RESULT': 'success',
                            'EXIT_CODE': 'exited', 'EXIT_STATUS': '0'})
        ended = json.loads((directory / 'session.json').read_text())
        result = {'directory': directory.name, 'errors': errors, 'slow': slow, 'painted': painted,
                  'started': {k: started.get(k) for k in ('pid', 'server', 'start', 'end')},
                  'ended': {k: ended.get(k) for k in ('end', 'seconds', 'status')}}
        # Named, so a failure says which.
        checks = {
            'two errors': len(errors) == 2,
            'first has a backtrace': bool(errors) and isinstance(errors[0].get('backtrace'), list)
                                     and 'logging-live-fail' in errors[0]['backtrace'],
            'second has none': len(errors) > 1 and 'backtrace' not in errors[1],
            'slow loop': bool(slow) and slow[0]['max'] >= 150 and slow[0]['ms'] >= 150,
            'slow redisplay': bool(painted) and painted[0]['ms'] < 50 and painted[0]['redisplay_ms'] >= 80
                              and painted[0]['max'] >= 80,
            'started record': started.get('server') == a.name and 'end' not in started,
            'own end written': isinstance(own_end.get('end'), (int, float)) and 'status' not in own_end,
            # The server's own end kept (serde_json reads a float to within
            # an ulp, not exactly, and writes it back so).
            'unit end filled': isinstance(ended.get('end'), (int, float)) and isinstance(own_end.get('end'), (int, float))
                               and abs(ended['end'] - own_end['end']) < 1e-3 and ended.get('status') == 0
                               and ended.get('service', {}).get('result') == 'success',
        }
        result['failed'] = [name for name, ok in checks.items() if not ok]
        if result['failed']:
            result['own_end_record'], result['ended_record'] = own_end, ended
        result['load'] = os.getloadavg()[0]
        result['ok'] = not result['failed']
        return result
    finally:
        a.close()


def composer_real(args, base):
    """`composer' with a real Claude Code (--claude-model): costs money."""
    return composer(args, base, real=True)


def composer(args, base, real=False):
    """Real keys typed into a chat's composer in the GUI: before the first
    prompt, after the first turn (the buffer renamed by it), DEL at the
    composer, and after M-> and a recenter in a long wrapped transcript;
    each time the composer holds exactly what was typed and no command
    signalled."""
    a = Agents(args, base, 'cm')
    try:
        a.up()
        a.work.mkdir(parents=True, exist_ok=True)
        if real:
            a.providers([('claude', args.claude_config, args.claude_program or real_program('claude'),
                          ['--model', args.claude_model])])
        a.keyboard_ready()
        time.sleep(1.5)  # the readiness probe's C-g presses, all taken
        sid = a.start('')
        chat = f'(romacs-agent-buffer {a.s(sid)})'
        a.eval(f'(progn (delete-other-windows) (romacs-agent-show {a.s(sid)}) t)')
        mark = a.eval('(with-current-buffer "*Messages*" (point-max))')

        def look():
            # The composer's text, point, the end marker, point-max, the
            # buffer's name, whether it is shown, the window's start, and
            # whether its text shows: any invisible character, its
            # properties, and its width as laid out in the window.
            return parity_live.read_sexp(a.eval(
                f'(with-current-buffer {chat} (let ((end (marker-position romacs-agent-ui--end))) '
                '(list (romacs-agent-ui--composer-text) (point) end (point-max) (buffer-name) '
                '(eq (window-buffer (selected-window)) (current-buffer)) (window-start (selected-window)) '
                '(cl-loop for p from end below (point-max) thereis (invisible-p p)) '
                '(and (< end (point-max)) (format "%S" (text-properties-at end))) '
                '(and (< end (point-max)) (car (window-text-pixel-size (selected-window) end (point-max)))))))'))

        def messages():
            return parity_live.read_sexp(a.eval(f'(with-current-buffer "*Messages*" '
                                                f'(buffer-substring-no-properties (min {mark} (point-max)) (point-max)))'))

        steps = {}

        def typed(name, text, expect=None):
            if text:
                a.keys(text)
            expect = text if expect is None else expect
            try:
                until(lambda: look()[0] == expect, 10, f'{name} typed')
            except RuntimeError:
                pass
            state = look()
            steps[name] = {'state': state, 'expect': expect}
            return state[0] == expect and not state[7] and state[8] in (None, 'nil') and (state[9] or 0) > 0

        def cleared(name):
            """Kill the composer's line, as a user would: C-a C-k."""
            a.keys('-M', 'ctrl', '-k', 'a', '-m', 'ctrl', '-M', 'ctrl', '-k', 'k', '-m', 'ctrl')
            try:
                until(lambda: look()[0] == '', 10, name)
            except RuntimeError:
                steps[name] = {'state': look(), 'expect': ''}
                # Cleared for the steps after, whatever the keys did.
                a.eval(f'(with-current-buffer {chat} (delete-region romacs-agent-ui--end (point-max)))')
                return False
            return True

        def turn(name):
            done = a.eval(f'(romacs-agent-done-seq {a.s(sid)})')
            a.keys('-k', 'Return')
            until(lambda: a.eval(f'(romacs-agent-done-seq {a.s(sid)})') != done, 120, f'{name} answered')
            a.idle(sid)

        good = [typed('before_first_prompt', 'say hello from Romacs')]
        turn('first')
        # A collapsed tool output is the transcript's last text while a turn
        # runs: typing then, and after the turn ends, shows.
        release = a.root / 'release-hold'
        a.send(sid, f'hold {release} romacs_eval {{"instance": "?"}}')
        # Held until released: the folded output stays the transcript's last text.
        until(lambda: a.eval(f'(with-current-buffer {chat} (and (get-text-property (1- romacs-agent-ui--end) '
                             '(quote romacs-agent-output)) t))') == 't', 15, 'tool output last')
        steps['output_last'] = a.eval(f'(with-current-buffer {chat} (format "%S" (text-properties-at (1- romacs-agent-ui--end))))')
        good.append(typed('while_output_last', 'typed while'))
        good.append(a.eval(f'(with-current-buffer {chat} (and (get-text-property (1- romacs-agent-ui--end) '
                           '(quote romacs-agent-output)) t))') == 't' and a.state(sid) == 'running')
        release.write_text('')
        a.idle(sid)
        good.append(typed('after_output_turn', ' and after', 'typed while and after'))
        good.append(cleared('composer cleared'))
        good.append(typed('after_first_turn', 'say again'))
        a.keys('-k', 'BackSpace', '-k', 'BackSpace')
        good.append(typed('after_del', 'x', 'say agax'))
        # emacs-hermes's chat keys and its actions menu, with real keys.
        ctrl = lambda key: ['-M', 'ctrl', '-k', key, '-m', 'ctrl']
        popup = lambda: a.eval('(and (boundp (quote keymap-popup--buffer)) (buffer-live-p keymap-popup--buffer) t)') == 't'
        good.append(cleared('composer cleared for keys'))
        a.keys('a'); a.keys(*ctrl('j')); a.keys('b')
        good.append(typed('newline', '', 'a\nb'))
        a.keys(*ctrl('c'), *ctrl('o'))
        try:
            until(popup, 10, 'the actions menu')
            steps['menu_opened'] = True
        except RuntimeError:
            steps['menu_opened'] = False
        a.keys('R')
        until(lambda: a.eval('(and (active-minibuffer-window) t)') == 't', 10, 'the rename prompt')
        a.keys(*ctrl('a'), *ctrl('k'))
        a.keys('menu title')
        a.keys('-k', 'Return')
        until(lambda: a.eval(f'(romacs-agent-title {a.s(sid)})') == '"menu title"', 10, 'renamed from the menu')
        steps['renamed'] = a.eval(f'(buffer-name (romacs-agent-buffer {a.s(sid)}))')
        good.append(steps['menu_opened'] and 'menu title' in steps['renamed'])
        a.eval(f'(with-current-buffer {chat} (delete-region romacs-agent-ui--end (point-max)))')
        # A message queued while a turn runs is sent when it ends.
        a.keys('slow 30'); a.keys('-k', 'Return')
        until(lambda: a.state(sid) in ('running', 'starting'), 10, 'the slow turn')
        a.keys('say queued one'); a.keys(*ctrl('c'), *ctrl('q'))
        steps['queued_composer'] = look()[0]
        until(lambda: 'queued one' in a.chat(sid).split('> say queued one', 1)[-1] and '> say queued one' in a.chat(sid),
              40, 'the queued message sent after the turn')
        a.idle(sid)
        good.append(steps['queued_composer'] == '')
        # Interrupt and send: the turn ends, then the composer's text is sent.
        a.keys('slow 200'); a.keys('-k', 'Return')
        until(lambda: a.state(sid) in ('running', 'starting'), 10, 'the long turn')
        a.keys('say after interrupt'); a.keys(*ctrl('c'), *ctrl('k'))
        until(lambda: '> say after interrupt' in a.chat(sid), 30, 'sent after the interrupt')
        a.idle(sid)
        steps['interrupted_then_sent'] = 'after interrupt' in a.chat(sid).split('> say after interrupt', 1)[-1]
        good.append(steps['interrupted_then_sent'])
        # M-p / M-n walk the composer's history; ? in the transcript opens the
        # menu, C-g closes it; w copies the turn at point.
        a.keys('-M', 'alt', '-k', 'p', '-m', 'alt')
        good.append(typed('history_previous', '', 'say after interrupt'))
        good.append(cleared('composer cleared for transcript keys'))
        a.keys('-M', 'alt', '<', '-m', 'alt')
        a.keys('?')
        try:
            until(popup, 10, 'the menu from the transcript')
            steps['menu_from_transcript'] = True
        except RuntimeError:
            steps['menu_from_transcript'] = False
        a.keys(*ctrl('g'))
        until(lambda: not popup(), 10, 'the menu closed')
        a.keys('w')
        steps['copied'] = parity_live.read_sexp(a.eval('(current-kill 0)'))[:40]
        good.append(steps['menu_from_transcript'] and (steps['copied'] or '').startswith('> say hello from Romacs'))
        a.keys('-M', 'alt', '>', '-m', 'alt')
        # The queue is the session's: sent though the chat buffer is gone, and
        # after a Codex turn an interrupt ended (its run exits, no result).
        a.send(sid, 'slow 30')
        until(lambda: a.state(sid) in ('running', 'starting'), 10, 'a turn to queue behind')
        a.eval(f'(progn (romacs-agent-send-later {a.s(sid)} "say queued unseen") '
               f'(let ((kill-buffer-query-functions nil)) (kill-buffer (romacs-agent-buffer {a.s(sid)}))) t)')
        until(lambda: 'queued unseen' in (a.results(sid) or [''])[-1], 40, 'the queued message sent with no chat')
        a.idle(sid)
        a.eval(f'(progn (save-window-excursion (romacs-agent-show {a.s(sid)})) '
               f'(delete-other-windows) (switch-to-buffer (romacs-agent-buffer {a.s(sid)})) (goto-char (point-max)) t)')
        a.codex_instance()
        codex = a.start('slow 30', 'codex')
        until(lambda: a.state(codex) in ('running', 'starting'), 15, 'a Codex turn')
        a.eval(f'(progn (romacs-agent-send-later {a.s(codex)} "say queued codex") (romacs-agent-interrupt {a.s(codex)}) t)')
        until(lambda: 'queued codex' in str(a.eval(f'(romacs-agent-reply {a.s(codex)})')), 40, 'the Codex queue after an interrupt')
        steps['queue_survives'] = True
        a.providers([('claude', '~/.claude')])
        # Non-ASCII typed with real keys, then sent: the harness gets it whole.
        good.append(cleared('composer cleared for unicode'))
        good.append(typed('unicode_typed', f'say {UNICODE}'))
        turn('unicode')
        steps['unicode_sent'] = UNICODE in a.chat(sid).rsplit(f'> say {UNICODE}', 1)[-1]
        good.append(steps['unicode_sent'])
        good.append(cleared('composer cleared for selection'))
        # With delete-selection-mode, a selection in the composer is
        # replaced by a typed key, as anywhere.
        a.keys('replace me')
        until(lambda: look()[0] == 'replace me', 10, 'selection text typed')
        a.keys('-M', 'ctrl', '-k', 'a', '-m', 'ctrl', '-M', 'ctrl', '-k', 'space', '-m', 'ctrl',
               '-M', 'ctrl', '-k', 'e', '-m', 'ctrl')
        a.eval('(progn (delete-selection-mode 1) t)')
        good.append(typed('selection_replaced', 'x', 'x'))
        # A region-using key (electric-pair's quote) wraps the selection, as
        # self-insert-command does.
        good.append(cleared('composer cleared for pairing'))
        a.keys('keep me')
        until(lambda: look()[0] == 'keep me', 10, 'pairing text typed')
        a.eval('(progn (electric-pair-mode 1) t)')
        a.keys('-M', 'ctrl', '-k', 'a', '-m', 'ctrl', '-M', 'ctrl', '-k', 'space', '-m', 'ctrl',
               '-M', 'ctrl', '-k', 'e', '-m', 'ctrl')
        good.append(typed('selection_wrapped', '"', '"keep me"'))
        a.eval('(progn (electric-pair-mode -1) (delete-selection-mode -1) t)')
        good.append(cleared('composer cleared for yank'))
        # Transcript text copied and yanked into the composer: plain text there.
        a.eval(f'(with-current-buffer {chat} (kill-new (buffer-substring (point-min) (+ (point-min) 5))) t)')
        yanked = parity_live.read_sexp(a.eval(f'(with-current-buffer {chat} (buffer-substring-no-properties (point-min) (+ (point-min) 5)))'))
        a.keys('-M', 'ctrl', '-k', 'y', '-m', 'ctrl')
        good.append(typed('yanked_transcript', '', yanked))
        steps['after_change_hook_kept'] = a.eval(f'(with-current-buffer {chat} (and (memq (function romacs-agent-ui--composer-plain) after-change-functions) t))')
        good.append(steps['after_change_hook_kept'] == 't')
        good.append(cleared('composer killed'))
        # A long wrapped transcript: replies far wider than the window.
        words = ' '.join(f'word{i}' for i in range(120))
        for _ in range(1 if real else 4):
            a.send(sid, f'say {words}' if not real else f'Repeat these words exactly, nothing else: {words}')
            a.idle(sid, 120)
        a.keys('-M', 'alt', '>', '-m', 'alt')
        good.append(typed('after_end_of_buffer', 'third'))
        a.keys('-M', 'ctrl', '-k', 'l', '-m', 'ctrl')
        good.append(typed('after_recenter', ' fourth', 'third fourth'))
        a.eval(f'(progn (romacs-agent-show {a.s(sid)}) t)')
        good.append(typed('after_show_again', ' fifth', 'third fourth fifth'))
        # Point in the transcript (M-<, as a scroll leaves it): a key typed
        # there goes to the composer (SPC there scrolls, as in hermes's
        # chat); M-> from there reaches it too.
        a.keys('-M', 'alt', '<', '-m', 'alt')
        good.append(typed('typed_in_transcript', '+sixth', 'third fourth fifth+sixth'))
        a.keys('-M', 'alt', '<', '-m', 'alt')
        a.keys('-M', 'alt', '>', '-m', 'alt')
        good.append(typed('after_top_then_end', ' seventh', 'third fourth fifth+sixth seventh'))
        good.append(cleared('composer killed again'))
        # A session started from the home page with c, its prompt typed in
        # the minibuffer: its chat is shown, and its composer takes keys.
        a.eval(f'(progn (delete-other-windows) (romacs-agents) (delete-other-windows) (cd {lisp(a.work)}) t)')
        before = set(parity_live.read_sexp(a.eval('(mapcar (function romacs-agent-id) (romacs-agent-sessions))')) or [])
        a.keys('c')
        until(lambda: a.eval('(and (minibufferp (window-buffer (minibuffer-window))) (active-minibuffer-window) t)') == 't',
              10, 'directory asked')
        a.keys('-k', 'Return')
        until(lambda: 'Prompt' in (parity_live.read_sexp(a.eval('(and (active-minibuffer-window) (minibuffer-prompt))')) or ''),
              10, 'prompt asked')
        a.keys('say hello from Romacs')
        a.keys('-k', 'Return')
        until(lambda: set(parity_live.read_sexp(a.eval('(mapcar (function romacs-agent-id) (romacs-agent-sessions))')) or []) - before,
              10, 'session started')
        sid = next(iter(set(parity_live.read_sexp(a.eval('(mapcar (function romacs-agent-id) (romacs-agent-sessions))'))) - before))
        chat = f'(romacs-agent-buffer {a.s(sid)})'
        until(lambda: a.eval(f'(romacs-agent-done-seq {a.s(sid)})') != '0', 120, 'home session answered')
        a.idle(sid)
        good.append(typed('home_session', 'say from home'))
        turn('home')
        good.append(typed('home_after_turn', 'more'))
        said = messages()
        result = {'steps': steps, 'messages': said[-1500:], 'title': look()[4]}
        result['ok'] = all(good) and not any(word in said for word in ('unsupported', 'read-only', 'error', 'Error'))
        return result
    finally:
        a.close()


def history_fixtures(claude_dir, codex_home, cwd):
    """A Claude Code transcript and a Codex rollout of a conversation in
    CWD, as the harnesses write them: an injected context before each first
    prompt, and Codex's long instructions before its own."""
    def lines(path, objects):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(''.join(json.dumps(o, separators=(',', ':')) + '\n' for o in objects))
    cid, tid = '11111111-2222-4333-8444-555555555555', '01a1a2ee-0000-7000-8000-000000000001'
    at = lambda s: f'2026-11-03T10:00:{s:02d}.000Z'
    user = lambda content, s, **extra: {'parentUuid': None, 'isSidechain': False, 'type': 'user', 'cwd': str(cwd),
                                        'message': {'role': 'user', 'content': content}, 'timestamp': at(s), **extra}
    said = lambda mid, block, s: {'isSidechain': False, 'type': 'assistant', 'timestamp': at(s),
                                  'message': {'model': 'claude-haiku-5-5', 'id': mid, 'type': 'message', 'role': 'assistant',
                                              'content': [block], 'usage': {'input_tokens': 10, 'cache_read_input_tokens': 2000}}}
    lines(claude_dir / 'projects' / str(cwd).replace('/', '-') / f'{cid}.jsonl', [
        {'type': 'queue-operation', 'operation': 'enqueue', 'timestamp': at(0), 'sessionId': cid},
        user('<command-name>/clear</command-name>', 0, isMeta=True),
        user('list the files here\nplease', 1, permissionMode='default'),
        said('msg_1', {'type': 'text', 'text': 'Sure, **listing** now.'}, 2),
        said('msg_1', {'type': 'tool_use', 'id': 'toolu_1', 'name': 'Bash', 'input': {'command': 'ls'}}, 3),
        user([{'type': 'tool_result', 'tool_use_id': 'toolu_1', 'content': 'a.txt\nb.txt'}], 4),
        said('msg_2', {'type': 'text', 'text': 'Two files: a.txt and b.txt.'}, 9)])
    item = lambda s, kind, payload: {'timestamp': at(s), 'type': kind, 'payload': payload}
    lines(codex_home / 'sessions/2026/11/03' / f'rollout-2026-11-03T10-00-00-{tid}.jsonl', [
        item(0, 'session_meta', {'id': tid, 'cwd': str(cwd), 'base_instructions': 'x' * 200000}),
        item(0, 'response_item', {'type': 'message', 'role': 'user',
                                  'content': [{'type': 'input_text', 'text': '# AGENTS.md instructions for here'}]}),
        item(0, 'turn_context', {'model': 'gpt-6-luna', 'cwd': str(cwd)}),
        item(1, 'event_msg', {'type': 'item_completed', 'thread_id': tid,
                              'item': {'type': 'UserMessage', 'content': [{'type': 'text', 'text': 'count the lines of a.txt'}]}}),
        item(1, 'response_item', {'type': 'reasoning', 'encrypted_content': 'zzz'}),
        item(2, 'response_item', {'type': 'function_call', 'name': 'exec_command', 'arguments': '{"cmd":"wc -l a.txt"}',
                                  'call_id': 'call_1'}),
        item(3, 'response_item', {'type': 'function_call_output', 'call_id': 'call_1', 'output': '3 a.txt'}),
        item(4, 'event_msg', {'type': 'token_count', 'info': {'last_token_usage': {'input_tokens': 4200},
                                                              'model_context_window': 272000}}),
        item(5, 'response_item', {'type': 'message', 'role': 'assistant',
                                  'content': [{'type': 'output_text', 'text': 'a.txt has 3 lines.'}]}),
        item(6, 'event_msg', {'type': 'task_complete'})])
    return cid, tid


def tree_state(*roots):
    return {str(p): (p.stat().st_size, p.stat().st_mtime_ns) for root in roots for p in sorted(root.rglob('*'))}


def history(args, base):
    """The harnesses' own sessions, as T3 Code imports them: a Claude Code
    transcript and a Codex rollout in private configuration directories are
    listed under their project, titled by the first typed prompt (injected
    context skipped, Codex's past a long head); opened read-only in the
    chat's look; resumed through the harness's own resume (claude --resume
    <id> in the instance's directory, codex exec resume <thread>), the
    transcript drawn before the session's own turns and no longer listed as
    history; after a restart both are sessions again.  Nothing is written
    under the configuration directories."""
    a = Agents(args, base, 'hi')
    try:
        a.up()
        claude_dir, codex_home = a.home / '.config/claude/acct', a.home / '.config/codex-acct'
        a.work.mkdir(parents=True, exist_ok=True)
        cid, tid = history_fixtures(claude_dir, codex_home, a.work)
        before = tree_state(claude_dir, codex_home)
        setting = [('claude', claude_dir), ('codex', codex_home, CODEX, [])]
        a.providers(setting)
        text = lambda form: parity_live.read_sexp(a.eval(form))
        home = text('(progn (save-window-excursion (romacs-agents)) (with-current-buffer "*romacs-agents*" '
                    '(buffer-substring-no-properties (point-min) (point-max))))')
        item = lambda ident: (f'(seq-find (lambda (i) (equal (plist-get i :id) {lisp_text(ident)})) '
                              '(romacs-agent-history-items))')
        views = {}
        for ident in (cid, tid):
            views[ident] = text(f'(with-current-buffer (save-window-excursion (romacs-agent-history-open {item(ident)})) '
                                '(format "%s|%s|%s" (buffer-name) buffer-read-only '
                                '(buffer-substring-no-properties (point-min) (point-max))))')
        resumed = {}
        for ident, prompt in ((cid, 'say resumed one'), (tid, 'say resumed two')):
            sid = a.eval(f'(with-current-buffer (save-window-excursion (romacs-agent-history-resume {item(ident)})) '
                         '(romacs-agent-id romacs-agent-ui--session))').strip('"')
            a.idle(sid) if ident == tid else until(
                lambda: a.eval(f'(and (process-live-p (romacs-agent-connection {a.s(sid)})) t)') == 't', 15, 'resumed')
            a.send(sid, prompt)
            until(lambda: prompt.removeprefix('say ') in a.chat(sid), 30, f'{prompt} answered')
            a.idle(sid)
            resumed[ident] = sid
        claude_run = [e for e in a.logged() if e.get('started') == cid]
        codex_run = [e for e in a.codex_runs() if e.get('codex') == tid]
        chats = {ident: a.chat(sid) for ident, sid in resumed.items()}
        listed = text('(progn (romacs-agent-history-list) (mapcar (lambda (i) (plist-get i :id)) (romacs-agent-history-items)))') or []
        a.client('--stop=none')
        until(lambda: not Path(f'/proc/{a.pid}').exists(), 15, 'server gone')
        a.restart()
        a.providers(setting)
        again = text(f'(progn (require \'romacs-agent-history) (romacs-agent-history-list) '
                     f'(list (mapcar (lambda (i) (plist-get i :id)) (romacs-agent-history-items)) '
                     f'(plist-get (romacs-agent-imported {a.s(resumed[cid])}) :path) '
                     f'(romacs-agent-harness-session {a.s(resumed[tid])})))')
        chat_again = a.chat(resumed[cid])
        untouched = tree_state(claude_dir, codex_home) == before
        # Another server's import of a conversation already going on finds its
        # session (the harness has since appended to the transcript: the cut stays).
        transcript = next((claude_dir / 'projects').rglob(f'{cid}.jsonl'))
        cut = transcript.stat().st_size
        with open(transcript, 'a') as handle:
            handle.write('{"type":"user","message":{"role":"user","content":"later"}}\n')
        imported_again = text(f'(let ((s (romacs-agent-import \'claude {lisp_text(cid)} {lisp_text(str(transcript))} '
                              f'{lisp(a.work)} "x" "claude" {lisp_text(str(claude_dir))}))) '
                              f'(list (eq s {a.s(resumed[cid])}) (plist-get (romacs-agent-imported s) :size)))')
        # A stale history row of a stopped session resumes it.
        a.eval(f'(romacs-agent-stop {a.s(resumed[cid])})')
        until(lambda: a.state(resumed[cid]) in ('ended', 'detached'), 15, 'stopped')
        runs = len([e for e in a.logged() if e.get('started') == cid])
        a.eval(f'(save-window-excursion (romacs-agent-history-resume (list :id {lisp_text(cid)} :harness \'claude '
               f':path {lisp_text(str(transcript))} :cwd {lisp(a.work)} :title "x" :provider "claude" '
               f':config-dir {lisp_text(str(claude_dir))})))')
        until(lambda: len([e for e in a.logged() if e.get('started') == cid]) > runs, 15, 'stale row resumed')
        # A conversation another server is importing right now is refused.
        busy = '01a1a2ee-0000-7000-8000-0000000000ff'
        # Its directory, named by the conversation's identity.
        busy_dir = text(f'(romacs-agent--import-id \'codex {lisp_text(str(codex_home))} {lisp_text(busy)})')
        (a.home / '.local/state/romacs/agents' / busy_dir).mkdir()
        refused = text(f'(condition-case err (romacs-agent-import \'codex {lisp_text(busy)} "/nonexistent" {lisp(a.work)} '
                       f'"x" "codex" {lisp_text(str(codex_home))}) (user-error (cadr err)))')
        # Two conversations whose time-ordered ids share their first
        # characters run overlapping turns, each with its own supervisor.
        threads = ['01a1a2ee-0000-7000-8000-0000000000a1', '01a1a2ee-0000-7000-8000-0000000000a2']
        twins = []
        for twin in threads:
            rollout = codex_home / 'sessions/2026/11/03' / f'rollout-2026-11-03T10-00-01-{twin}.jsonl'
            rollout.write_text(json.dumps({'timestamp': '2026-11-03T10:00:01.000Z', 'type': 'session_meta',
                                           'payload': {'id': twin, 'cwd': str(a.work)}}) + '\n')
            twins.append(text(f'(romacs-agent-id (romacs-agent-import \'codex {lisp_text(twin)} {lisp_text(str(rollout))} '
                              f'{lisp(a.work)} "twin" "codex" {lisp_text(str(codex_home))}))'))
        sockets = text(f'(list (romacs-agent-host-socket {a.s(twins[0])}) (romacs-agent-host-socket {a.s(twins[1])}))')
        for twin in twins:
            a.send(twin, 'slow 2')
        for twin in twins:
            until(lambda: a.eval(f'(romacs-agent-done-seq {a.s(twin)})') != '0', 30, f'{twin} answered')
        twin_runs = sorted({e.get('codex') for e in a.codex_runs() if e.get('codex') in threads})
        twin_failed = text(f'(list (romacs-agent-failed {a.s(twins[0])}) (romacs-agent-failed {a.s(twins[1])}))')
        # Byte ranges: a line starting at BEG is whole; one cut at BEG (inside
        # a UTF-8 character) or at END is left out.
        ranges = a.work / 'ranges.jsonl'
        rows = ['{"type":"a"}\n', '{"type":"b","t":"é"}\n', '{"type":"c"}\n']
        ranges.write_bytes(''.join(rows).encode())
        b0, b1 = len(rows[0].encode()), len(rows[0].encode()) + len(rows[1].encode())
        read = lambda beg, end: text(f'(mapcar (lambda (o) (gethash "type" o)) (romacs-agent-history--read '
                                     f'{lisp_text(str(ranges))} {beg} {end} (lambda (&rest _) t)))') or []
        cuts = {'aligned': read(b0, b1 + len(rows[2])), 'in-char': read(b0 + 18, b1 + len(rows[2])),
                'partial-end': read(0, b1 + 3), 'whole': read(0, b1 + len(rows[2]))}
        order = lambda chat, first, then: first in chat and then in chat and chat.index(first) < chat.index(then)
        result = {
            'home': home, 'views': {k: v[:400] for k, v in views.items()},
            'claude_argv': claude_run[-1]['argv'] if claude_run else None,
            'claude_config_dir': claude_run[-1].get('config_dir') if claude_run else None,
            'codex_argv': codex_run[-1]['argv'] if codex_run else None,
            'chats': {k: v[:500] for k, v in chats.items()}, 'listed_after': listed, 'after_restart': again,
            'untouched': untouched, 'imported_again': imported_again, 'refused': refused, 'cuts': cuts,
            'twin_sockets': sockets, 'twin_runs': twin_runs, 'twin_failed': twin_failed}
        result['ok'] = bool(
            '◇ list the files here' in home and '◇ count the lines of a.txt' in home and 'AGENTS.md' not in home
            and home.index(a.work.name) < home.index('◇')
            and all(v.split('|')[1] == 't' for v in views.values())
            and views[cid].startswith('*history: list the files here*')
            and order(views[cid], '> list the files here', 'Two files') and '✓ Bash  ls' in views[cid]
            and order(views[tid], '> count the lines of a.txt', 'a.txt has 3 lines') and '✓ Bash  wc -l a.txt' in views[tid]
            and claude_run and claude_run[-1]['argv'][claude_run[-1]['argv'].index('--resume') + 1] == cid
            and claude_run[-1].get('config_dir') == str(claude_dir)
            and codex_run and codex_run[-1]['argv'][codex_run[-1]['argv'].index('resume') + 1] == tid
            and order(chats[cid], '> list the files here', '> say resumed one')
            and order(chats[tid], '> count the lines of a.txt', '> say resumed two')
            and cid not in listed and tid not in listed
            and again[0] in (None, []) and again[1].endswith(f'{cid}.jsonl') and again[2] == tid
            and order(chat_again, '> list the files here', '> say resumed one')
            and imported_again == ['t', cut] and 'Another Romacs' in (refused or '')
            and cuts == {'aligned': ['b', 'c'], 'in-char': ['c'], 'partial-end': ['a', 'b'], 'whole': ['a', 'b', 'c']}
            and sockets[0] != sockets[1] and twin_runs == threads and twin_failed in (None, [None, None], [])
            and untouched)
        return result
    finally:
        a.close()


def real_program(name):
    """NAME's real executable on PATH, past version-manager shims and
    wrappers, which need the real HOME's configuration."""
    for directory in os.environ.get('PATH', '').split(':'):
        candidate = Path(directory) / name
        if (not any(part in directory for part in ('command-wrappers', '/shims'))
                and candidate.is_file() and os.access(candidate, os.X_OK)):
            return str(candidate.resolve())
    return name


def real(args, base):
    """A real Claude Code session (--claude-model, default haiku) in an
    isolated Romacs calls every Romacs MCP tool, each result checked, and
    has a real Codex child (--codex-model, default gpt-6-luna) review a
    tiny diff through romacs_delegate_task.  Costs money: not a default
    workflow.  The instances use the real configuration directories
    (--claude-config, --codex-home) for their logins."""
    a = Agents(args, base, 'rl')
    try:
        a.up()
        a.providers([('claude', args.claude_config, args.claude_program or real_program('claude'), ['--model', args.claude_model]),
                     ('codex', args.codex_home, args.codex_program or real_program('codex'), [])])
        tour_setup(a)
        target = {'provider': 'codex', 'model': args.codex_model, 'options': {'reasoningEffort': args.codex_effort}}
        steps = tour(a.work, {'task': REVIEW, 'target': target})
        plan = '\n'.join(f'{n}. {tool} {json.dumps(arguments)}' for n, (tool, arguments, _) in enumerate(steps, 1))
        sid = a.start('Using only the romacs MCP tools, make these calls in this order, each exactly once, '
                      'with exactly these arguments (fill a taskId from the delegate call\'s reply); do not stop '
                      'at an error. Then reply DONE.\n' + plan)

        def allow():
            a.eval(f'(let ((s {a.s(sid)})) (dolist (id (hash-table-keys (romacs-agent-approvals s))) '
                   '(romacs-agent-respond s id t)) t)')
            return a.state(sid)
        until(lambda: allow() == 'idle', 900, 'the tour')
        uses, outputs = {}, {}
        for record in a.journal(sid):
            out = record.get('out') if isinstance(record.get('out'), dict) else {}
            content = (out.get('message') or {}).get('content')
            if out.get('type') == 'assistant' and isinstance(content, list):
                for block in content:
                    if block.get('type') == 'tool_use':
                        uses[block['id']] = block['name'].removeprefix('mcp__romacs__')
            if out.get('type') == 'user' and isinstance(content, list):
                for block in content:
                    if block.get('type') == 'tool_result':
                        text = block.get('content')
                        text = ''.join(c.get('text', '') for c in text) if isinstance(text, list) else str(text)
                        outputs.setdefault(uses.get(block['tool_use_id']), (text, block.get('is_error')))
        checked = {tool: tool in outputs and not outputs[tool][1] and check(outputs[tool][0]) for tool, _, check in steps}
        missing = sorted(set(offered(a)) - set(checked) - set(ELSEWHERE) - {t for t in offered(a) if t.startswith('gnosis_')})
        cost = [r['out'].get('total_cost_usd') for r in a.journal(sid)
                if isinstance(r.get('out'), dict) and r['out'].get('type') == 'result']
        review = outputs.get('romacs_delegate_task', ('', None))[0]
        result = {'checked': checked, 'not_in_tour': missing, 'review': review[:600], 'claude_cost': cost,
                  'failed': {t: outputs.get(t, ('missing',))[0][:200] for t, ok in checked.items() if not ok}}
        result['ok'] = all(checked.values()) and not missing
        return result
    finally:
        a.close()


def home_keys(args, base):
    """RET on the home page's lines that are no row, as real keys: on a
    project, its newest session (by last activity, not by start, a
    delegated one too) or, with
    none, its newest transcript opens; on "◇ N more", every transcript is
    shown; elsewhere a hint says n and p reach a row, and nothing opens; a
    project whose newest session is gone since the page was drawn says so
    and draws it again."""
    a = Agents(args, base, 'hk')
    try:
        a.up()
        claude_dir, codex_home = a.home / '.config/claude/acct', a.home / '.config/codex-acct'
        a.work.mkdir(parents=True, exist_ok=True)
        transcripts = a.root / 'transcripts'
        transcripts.mkdir()
        cid, tid = history_fixtures(claude_dir, codex_home, transcripts)
        claude_file = next((claude_dir / 'projects').rglob(f'{cid}.jsonl'))
        codex_file = next(codex_home.rglob(f'*{tid}.jsonl'))
        now = time.time()
        os.utime(claude_file, (now - 100, now - 100))
        os.utime(codex_file, (now - 50, now - 50))
        a.providers([('claude', claude_dir), ('codex', codex_home, CODEX, [])])
        sids = []
        for prompt in ('say one', 'say two', 'say three'):
            sid = a.start(prompt)
            until(lambda: a.eval(f'(romacs-agent-done-seq {a.s(sid)})') != '0', 30, f'{prompt} answered')
            a.idle(sid)
            sids.append(sid)
        # The third, delegated by the first (its row under the first's), is
        # the most recently active; then the second.
        a.eval(f'(progn (setf (romacs-agent-options {a.s(sids[2])}) '
               f'(plist-put (romacs-agent-options {a.s(sids[2])}) :parent {lisp_text(sids[0])})) t)')
        for sid, at in zip(sids, (now - 30, now - 20, now - 10)):
            a.eval(f'(progn (setf (romacs-agent-last-time {a.s(sid)}) {at}) t)')
        a.eval('(progn (setq romacs-agent-history-per-project 1) (delete-other-windows) (romacs-agents) '
               '(delete-other-windows) t)')
        a.keyboard_ready()
        shown = lambda: a.eval('(buffer-name (window-buffer (selected-window)))').strip('"')
        # The session whose chat the selected window shows, if any.
        showing = lambda: parity_live.read_sexp(a.eval(
            '(with-current-buffer (window-buffer (selected-window)) '
            '(and (bound-and-true-p romacs-agent-ui--session) (romacs-agent-id romacs-agent-ui--session)))'))

        def ret_on(regexp):
            """Point to the home page's line REGEXP matches; RET there."""
            a.eval('(progn (switch-to-buffer "*romacs-agents*") (delete-other-windows) (goto-char (point-min)) '
                   f'(re-search-forward {lisp_text(regexp)}) (beginning-of-line) t)')
            a.keys('-k', 'Return')
            time.sleep(1)

        steps = {}
        ret_on('^transcripts  ')
        steps['project_without_session'] = parity_live.read_sexp(a.eval(
            '(with-current-buffer (window-buffer (selected-window)) '
            '(list buffer-read-only (plist-get romacs-agent-history--item :path)))'))
        ret_on('^work  ')
        steps['project_with_session'] = showing()
        steps['nested'] = parity_live.read_sexp(a.eval(
            f'(mapcar (function romacs-agent-id) (gethash {lisp_text(sids[0])} (cdr (romacs-agent-ui--tree))))'))
        ret_on('◇ [0-9]+ more')
        steps['more'] = [shown(), a.eval('romacs-agents--all-history'),
                         a.eval('(with-current-buffer "*romacs-agents*" (and (string-search "more  (h shows all)" (buffer-string)) t))')]
        ret_on('^RET open')
        steps['help_line'] = [shown(), parity_live.read_sexp(a.eval('(current-message)'))]
        # The newest session forgotten after the page was drawn.
        a.eval(f'(progn (remhash {lisp_text(sids[2])} romacs-agent--sessions) t)')
        ret_on('^work  ')
        steps['gone'] = [shown(), parity_live.read_sexp(a.eval('(current-message)'))]
        ret_on('^work  ')
        steps['after_gone'] = showing()
        result = {'steps': steps, 'sessions': sids, 'codex_file': str(codex_file)}
        result['ok'] = (steps['project_without_session'] == ['t', str(codex_file)]
                        and steps['project_with_session'] == sids[2] and steps['nested'] == [sids[2]]
                        and steps['more'] == ['*romacs-agents*', 't', 'nil']
                        and steps['help_line'][0] == '*romacs-agents*'
                        and 'n and p' in (steps['help_line'][1] or '')
                        and steps['gone'][0] == '*romacs-agents*' and 'gone' in (steps['gone'][1] or '')
                        and steps['after_gone'] == sids[1])
        return result
    finally:
        a.close()


def start_picker(args, base):
    """Starting a session by real keys through the palette: C-u c on the
    home page, the directory of the project at point, a row found by typing
    (its instance's group, the harness's reported model among the catalogue's),
    the effort, the prompt.  The fake harnesses record what they were given.
    The first session of an instance asks its models once and keeps only
    their rows; with one enabled instance no palette is asked."""
    a = Agents(args, base, 'pk')
    try:
        a.up()
        a.work.mkdir(parents=True, exist_ok=True)
        # A repository's subdirectory: `c' there offers it, not the root.
        (a.root / 'repository/.git').mkdir(parents=True)
        other = a.root / 'repository/other'
        other.mkdir()
        a.codex_instance()
        kept = a.home / '.local/state/romacs/agents/models/claude.json'

        first = a.start('say one')
        until(lambda: kept.exists(), 30, 'models kept')
        a.idle(first)
        second = a.eval(f'(romacs-agent-id (romacs-agent-start {lisp(other)} "say two"))').strip('"')
        a.idle(second)
        result = {'asked': sum(1 for e in a.logged() if e.get('control') == 'initialize'),
                  'kept': kept.read_text()}
        kept_ok = 'claude-fake-1' in result['kept'] and 'someone@example.org' not in result['kept']

        a.eval('(progn (delete-other-windows) (romacs-agents) (delete-other-windows) t)')
        a.keyboard_ready()

        def prompt():
            return parity_live.read_sexp(a.eval(
                '(when (active-minibuffer-window) (with-current-buffer (window-buffer (active-minibuffer-window)) '
                '(substring-no-properties (minibuffer-prompt))))')) or ''

        def contents():
            return parity_live.read_sexp(a.eval(
                '(with-current-buffer (window-buffer (active-minibuffer-window)) (minibuffer-contents-no-properties))'))

        def asked(text):
            until(lambda: text in prompt(), 15, f'{text} asked')

        def on_project(path):
            a.eval('(progn (switch-to-buffer "*romacs-agents*") (delete-other-windows) (goto-char (point-min)) '
                   f'(re-search-forward {lisp_text("^" + path.name + "  ")}) (beginning-of-line) t)')

        def pick(project, typed, effort, text):
            """C-u c at PROJECT's section; the row TYPED finds, at EFFORT."""
            on_project(project)
            a.keys('-M', 'ctrl', '-k', 'u', '-m', 'ctrl')
            a.keys('c')
            asked('Project directory')
            directory = contents()
            a.keys('-k', 'Return')
            asked('Agent')
            a.keys(typed)
            time.sleep(.5)
            a.keys('-k', 'Return')
            asked('Effort')
            a.keys(effort)
            time.sleep(.5)
            a.keys('-k', 'Return')
            asked('Prompt')
            a.keys(text)
            a.keys('-k', 'Return')
            return directory

        # The palette, as drawn.
        on_project(other)
        a.keys('c')
        asked('Project directory')
        a.keys('-k', 'Return')
        asked('Agent')
        time.sleep(1)
        rows = parity_live.read_sexp(a.eval(
            '(mapcar (function car) (romacs-agent--picker-rows))'))
        shot = base / 'start-picker.png'
        subprocess.run([f'{args.tools}/grim', str(shot)], env=a.env, check=True, timeout=30)
        result['screenshot'] = str(shot)
        a.keys('-M', 'ctrl', '-k', 'g', '-m', 'ctrl')
        until(lambda: not prompt(), 15, 'palette closed')

        result['directory'] = pick(other, 'son', 'high', 'say sonnet')
        until(lambda: any(e.get('argv') and '--model' in e['argv'] for e in a.logged()), 30, 'claude started')
        claude_argv = next(e['argv'] for e in a.logged() if e.get('argv') and '--model' in e['argv'])
        pick(other, 'cod luna', 'low', 'say luna')
        until(lambda: any('gpt-6-luna' in r['argv'] for r in a.codex_runs()), 30, 'codex ran')
        codex_argv = next(r['argv'] for r in a.codex_runs() if 'gpt-6-luna' in r['argv'])
        result.update(claude_argv=claude_argv, codex_argv=codex_argv, rows=rows)

        # One enabled instance: from the directory straight to the prompt.
        a.eval('(progn (setq romacs-agent-providers (list (cons "claude" "~/.claude"))) t)')
        on_project(other)
        a.keys('c')
        asked('Project directory')
        a.keys('-k', 'Return')
        asked('Prompt')
        result['single'] = prompt()
        a.keys('-M', 'ctrl', '-k', 'g', '-m', 'ctrl')

        after = lambda argv, flag: argv[argv.index(flag) + 1] if flag in argv else None
        result['ok'] = (result['asked'] == 1 and kept_ok
                        and Path(result['directory'].rstrip('/')).name == other.name
                        and after(claude_argv, '--model') == 'claude-sonnet-5-5'
                        and after(claude_argv, '--effort') == 'high'
                        and after(codex_argv, '-m') == 'gpt-6-luna'
                        and 'model_reasoning_effort="low"' in codex_argv
                        and rows[0] == 'Claude · default' and 'Claude · Fake 1' in rows
                        and 'Codex · GPT-6.1 Sol' in rows and 'Prompt' in result['single'])
        return result
    finally:
        a.close()


def history_real(args, base):
    """A real Claude Code conversation (--claude-model) and a real Codex one
    (--codex-model), each made outside Romacs in the real configuration
    directories, are listed on the home page, resumed through the harness's
    own resume, and asked what they were told before.  Costs money: not a
    default workflow.  Leaves the two conversations in those directories."""
    a = Agents(args, base, 'hr')
    try:
        a.up()
        a.work.mkdir(parents=True, exist_ok=True)
        claude = args.claude_program or real_program('claude')
        codex = args.codex_program or real_program('codex')
        env = {k: v for k, v in os.environ.items() if k not in ('CLAUDE_CONFIG_DIR', 'CODEX_HOME')}
        made = subprocess.run([claude, '-p', '--model', args.claude_model, '--output-format', 'json',
                               'Remember the word PAPAYA. Reply only OK.'],
                              cwd=a.work, env={**env, 'CLAUDE_CONFIG_DIR': args.claude_config},
                              capture_output=True, text=True, timeout=300, stdin=subprocess.DEVNULL)
        cid = json.loads(made.stdout)['session_id']
        made = subprocess.run([codex, 'exec', '--json', '--skip-git-repo-check', '-m', args.codex_model,
                               '-c', f'model_reasoning_effort="{args.codex_effort}"', 'Remember the word MANGO. Reply only OK.'],
                              cwd=a.work, env={**env, 'CODEX_HOME': args.codex_home},
                              capture_output=True, text=True, timeout=300, stdin=subprocess.DEVNULL)
        tid = next(json.loads(line)['thread_id'] for line in made.stdout.splitlines()
                   if line.startswith('{') and json.loads(line).get('type') == 'thread.started')
        a.providers([('claude', args.claude_config, claude, ['--model', args.claude_model]),
                     ('codex', args.codex_home, codex, ['-m', args.codex_model,
                                                        '-c', f'model_reasoning_effort="{args.codex_effort}"'])])
        home = parity_live.read_sexp(a.eval('(progn (save-window-excursion (romacs-agents)) (with-current-buffer "*romacs-agents*" '
                                            '(buffer-substring-no-properties (point-min) (point-max))))'))
        answers, ids = {}, {}
        for ident, word in ((cid, 'PAPAYA'), (tid, 'MANGO')):
            sid = a.eval(f'(with-current-buffer (save-window-excursion (romacs-agent-history-resume '
                         f'(seq-find (lambda (i) (equal (plist-get i :id) {lisp_text(ident)})) (romacs-agent-history-items)))) '
                         '(romacs-agent-id romacs-agent-ui--session))').strip('"')
            ids[ident] = sid
            if ident == cid:
                until(lambda: a.eval(f'(and (process-live-p (romacs-agent-connection {a.s(sid)})) t)') == 't', 60, 'resumed')
            a.send(sid, 'What word did I ask you to remember? Answer with the word only.')
            until(lambda: a.eval(f'(romacs-agent-done-seq {a.s(sid)})') != '0', 300, f'{word} answered')
            a.idle(sid, 60)
            answers[word] = a.eval(f'(romacs-agent-reply {a.s(sid)})')
        chats = {word: a.chat(ids[ident])[:600] for ident, word in ((cid, 'PAPAYA'), (tid, 'MANGO'))}
        cost = [r['out'].get('total_cost_usd') for r in a.journal(ids[cid])
                if isinstance(r.get('out'), dict) and r['out'].get('type') == 'result']
        result = {'claude_session': cid, 'codex_thread': tid, 'answers': answers, 'chats': chats, 'claude_cost': cost,
                  'listed': [line for line in home.splitlines() if 'Remember the word' in line]}
        result['ok'] = (len(result['listed']) == 2 and 'PAPAYA' in answers['PAPAYA'] and 'MANGO' in answers['MANGO']
                        and all(c.index('> Remember the word') < c.index('> What word') for c in chats.values()))
        return result
    finally:
        a.close()


WORKFLOWS = {'host': host, 'deadline': deadline, 'providers': providers, 'session': session, 'survive': survive, 'outcome': outcome, 'budget': budget, 'unit': unit, 'probe': probe,
             'tools': tools, 'delegate': delegate, 'real': real, 'restart': restart, 'restart-unit': restart_unit, 'restart-race': restart_race,
             'gnosis': gnosis, 'history': history, 'history-real': history_real, 'composer': composer,
             'composer-real': composer_real, 'launcher': launcher, 'unicode': unicode,
             'environment': environment, 'logging': logging, 'home_keys': home_keys, 'start_picker': start_picker, 'rpc': rpc,
             'instances': instances, 'mcp_tour': mcp_tour}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--tools', default=parity_live.TOOLS)
    parser.add_argument('--emacsclient', default='emacsclient')
    parser.add_argument('--workflow', action='append', choices=sorted(WORKFLOWS))
    # The real-harness workflow's models: cheap by default; real reviews use Sol, high.
    parser.add_argument('--claude-model', default='haiku')
    parser.add_argument('--codex-model', default='gpt-6-luna')
    parser.add_argument('--codex-effort', default='low')
    parser.add_argument('--claude-config', default=os.environ.get('CLAUDE_CONFIG_DIR') or os.path.expanduser('~/.claude'))
    parser.add_argument('--codex-home', default=os.environ.get('CODEX_HOME') or os.path.expanduser('~/.codex'))
    parser.add_argument('--claude-program')
    parser.add_argument('--codex-program')
    args = parser.parse_args()
    out = Path(args.output)
    out.mkdir(parents=True, exist_ok=False)
    results = {}
    for name in args.workflow or ['host', 'session', 'composer', 'providers', 'tools', 'delegate', 'restart', 'gnosis', 'history', 'launcher', 'unicode', 'environment', 'logging', 'home_keys', 'rpc', 'instances', 'mcp_tour', 'survive',
                                  'outcome', 'budget', 'deadline', 'probe']:
        try:
            results[name] = WORKFLOWS[name](args, out)
        except Exception as error:  # a workflow's failure is its result
            results[name] = {'ok': False, 'error': repr(error)}
        print(name, json.dumps(results[name]), flush=True)
    (out / 'summary.json').write_text(json.dumps(results, indent=2))
    return 0 if all(r.get('ok') is not False for r in results.values()) else 1


if __name__ == '__main__':
    sys.exit(main())
