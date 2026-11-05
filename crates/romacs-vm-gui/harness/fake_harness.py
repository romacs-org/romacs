#!/usr/bin/env python3
"""A stand-in for Claude Code's stream-json CLI (`claude -p --input-format
stream-json --output-format stream-json'), for agent session tests
(agent_live.py).  It writes the same line types Claude Code writes, runs the
Romacs MCP server its --mcp-config names (the reconnecting relay) and calls
its tools as a real harness does, and answers control requests.  Its
"model" is a script: each user message is one command.

  say TEXT           stream TEXT as the reply, word by word
  slow N             stream N numbered lines, 0.1 s apart (a long turn)
  call TOOL JSON     call Romacs tool TOOL with JSON arguments ("instance":"?"
                     is filled from romacs_identity); on `outcome unknown' ask
                     romacs_call_outcome and repeat the call only when it says
                     `not executed'
  hold FILE TOOL JSON
                     call TOOL as `call' does, then wait until FILE exists
                     (60 s at most) before replying: its result is the
                     transcript's last text until the test releases it
  ask COMMAND        ask the host to approve Bash COMMAND (can_use_tool) and
                     report its answer
  later SECONDS CMD  run CMD after SECONDS
  spawn FILE         start a detached grandchild that appends to FILE every
                     0.2 s (the unit's cgroup must end it)

Every tool call it makes is logged to $FAKE_HARNESS_LOG as one JSON line.
"""
import json
import os
import queue
import subprocess
import sys
import threading
import time

from romacs_mcp_client import RomacsMcp

OUT = threading.Lock()
LOG = os.environ.get('FAKE_HARNESS_LOG')
# The variables whose inheritance the environment tests watch.
WATCHED = ('ROMACS_', 'CLAUDE', 'CODEX_', 'T3CODE_')


def emit(obj):
    with OUT:
        sys.stdout.write(json.dumps(obj) + '\n')
        sys.stdout.flush()


def log(**fields):
    if LOG:
        with open(LOG, 'a') as handle:
            handle.write(json.dumps({'t': time.time(), 'pid': os.getpid(), **fields}) + '\n')


def argument(name):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else None


def evaluator_reply(prompt):
    """A model's answer to Gnosis's evaluator or tutor prompt, or None:
    `pass' unless the response says MALFORMED, then not JSON."""
    if prompt.startswith('Evaluate the learner response'):
        if 'MALFORMED' in prompt:
            return 'certainly, here is my verdict: pass'
        return json.dumps({'verdict': 'pass', 'explanation': 'The essentials are there.'})
    if prompt.startswith('You are a study tutor'):
        return json.dumps({'verdict': 'pass', 'explanation': 'Good.'})
    return None


class Harness:
    def __init__(self):
        self.session = argument('--session-id') or argument('--resume')
        self.mcp = self.connect(argument('--mcp-config')) if argument('--mcp-config') else None
        self.inbox = queue.Queue()
        self.interrupted = threading.Event()
        self.approvals = {}
        self.turn = 0

    @staticmethod
    def connect(config_file):
        """The MCP client: the relay the config names, as Claude Code runs it."""
        server = json.load(open(config_file))['mcpServers']['romacs']
        return RomacsMcp([server['command'], *server['args']], name='fake-harness')

    def init(self):
        emit({'type': 'system', 'subtype': 'init', 'session_id': self.session, 'model': 'fake-model',
              'cwd': os.getcwd(), 'tools': ['Bash'], 'mcp_servers': [{'name': 'romacs', 'status': 'connected'}]})

    def stream(self, text, parent=None):
        self.turn += 1
        message_id = f'msg_{os.getpid()}_{self.turn}'
        emit({'type': 'stream_event', 'event': {'type': 'message_start', 'message': {'id': message_id}},
              'parent_tool_use_id': parent})
        for word in text.split(' '):
            emit({'type': 'stream_event', 'parent_tool_use_id': parent,
                  'event': {'type': 'content_block_delta', 'delta': {'type': 'text_delta', 'text': word + ' '}}})
        emit({'type': 'assistant', 'parent_tool_use_id': parent,
              'message': {'id': message_id, 'content': [{'type': 'text', 'text': text}]}})

    def result(self, text, error=False):
        emit({'type': 'result', 'subtype': 'error_during_execution' if error else 'success', 'is_error': error,
              'result': text, 'session_id': self.session, 'total_cost_usd': 0.0})

    def call(self, name, arguments):
        if arguments.get('instance') == '?':
            arguments['instance'] = self.mcp.identity()
        use = f'toolu_{os.getpid()}_{time.time_ns()}'
        emit({'type': 'assistant', 'parent_tool_use_id': None,
              'message': {'id': use, 'content': [{'type': 'tool_use', 'id': use, 'name': f'mcp__romacs__{name}',
                                                  'input': arguments}]}})
        started = time.time()
        ident, text, error = self.mcp.tool(name, arguments)
        log(tool=name, id=ident, text=text[:400], error=error, seconds=time.time() - started)
        if 'outcome unknown' in text:
            # Ask until a Romacs answers: while none listens the question
            # itself is `not executed'.
            for _ in range(60):
                _, outcome, _ = self.mcp.tool('romacs_call_outcome', {'id': ident})
                if 'not executed' not in outcome:
                    break
                time.sleep(.5)
            log(outcome_of=ident, outcome=outcome)
            if 'Romacs instance mismatch' in outcome or '"not executed"' in outcome:
                self.mcp.instance = None
            text = f'{text}\noutcome: {outcome}'
        elif 'Romacs instance mismatch' in text:
            # A new Romacs: its identity, then the one retry the tools allow.
            self.mcp.instance = None
            arguments['instance'] = '?'
            return self.call(name, arguments)
        elif 'not executed: Romacs is not running' in text:
            time.sleep(1)
            ident, text, error = self.mcp.tool(name, arguments)
            log(tool=name, id=ident, text=text[:400], error=error, repeated=True)
        emit({'type': 'user', 'parent_tool_use_id': None,
              'message': {'content': [{'type': 'tool_result', 'tool_use_id': use, 'content': text, 'is_error': error}]}})
        return text

    def run(self, text):
        self.interrupted.clear()
        verb, _, rest = text.partition(' ')
        if verb == 'say':
            self.stream(rest)
            return self.result(rest)
        if verb == 'slow':
            for n in range(int(rest)):
                if self.interrupted.is_set():
                    return self.interrupt_done()
                self.stream(f'line {n + 1}')
                time.sleep(.1)
            return self.result('slow done')
        if verb == 'call':
            name, _, raw = rest.partition(' ')
            reply = self.call(name, json.loads(raw or '{}'))
            self.stream(f'called {name}')
            return self.result(reply)
        if verb == 'hold':
            release, _, rest = rest.partition(' ')
            name, _, raw = rest.partition(' ')
            reply = self.call(name, json.loads(raw or '{}'))
            deadline = time.time() + 60
            while not os.path.exists(release) and time.time() < deadline:
                time.sleep(.05)
            self.stream(f'held {name}')
            return self.result(reply)
        if verb == 'ask':
            request_id = f'req-{time.time_ns()}'
            emit({'type': 'control_request', 'request_id': request_id,
                  'request': {'subtype': 'can_use_tool', 'tool_name': 'Bash', 'display_name': 'Bash',
                              'input': {'command': rest, 'run_in_background': False}, 'description': 'run ' + rest,
                              'tool_use_id': 'toolu_ask'}})
            answer = self.approvals_wait(request_id)
            log(approval=request_id, answer=answer)
            self.stream(f"approval {answer.get('behavior')}")
            return self.result(json.dumps(answer))
        if verb == 'later':
            seconds, _, command = rest.partition(' ')
            time.sleep(float(seconds))
            return self.run(command)
        if text.startswith('Delegated task ') or text.startswith('Romacs restart'):
            # A delegated task's end, or a restart, as Romacs tells a session.
            log(wake=text[:400])
            self.stream('woken')
            return self.result(text)
        reply = evaluator_reply(text)
        if reply is not None:
            log(evaluated=text[:200])
            self.stream(reply)
            return self.result(reply)
        if verb == 'spawn':
            child = subprocess.Popen(['sh', '-c', f'while true; do date +%s.%N >> {rest}; sleep 0.2; done'],
                                     start_new_session=True)
            log(grandchild=child.pid)
            self.stream(f'spawned {child.pid}')
            return self.result(str(child.pid))
        self.stream('unknown command')
        return self.result('unknown command', error=True)

    def interrupt_done(self):
        emit({'type': 'user', 'parent_tool_use_id': None,
              'message': {'content': [{'type': 'text', 'text': '[Request interrupted by user]'}]}})
        self.result('interrupted', error=True)

    def approvals_wait(self, request_id):
        while True:
            answer = self.approvals.get(request_id)
            if answer is not None:
                return answer
            time.sleep(.05)

    def read_stdin(self):
        for line in sys.stdin:
            message = json.loads(line)
            if message.get('type') == 'control_request':
                emit({'type': 'control_response', 'response': {'subtype': 'success', 'request_id': message['request_id'],
                                                               'response': {'still_queued': []}}})
                if message['request'].get('subtype') == 'interrupt':
                    self.interrupted.set()
            elif message.get('type') == 'control_response':
                response = message['response']
                self.approvals[response['request_id']] = response.get('response', {})
            elif message.get('type') == 'user':
                content = message['message']['content']
                self.inbox.put(content if isinstance(content, str) else ''.join(c.get('text', '') for c in content))
        self.inbox.put(None)

    def main(self):
        threading.Thread(target=self.read_stdin, daemon=True).start()
        log(started=self.session, argv=sys.argv[1:], config_dir=os.environ.get('CLAUDE_CONFIG_DIR'),
            cwd=os.getcwd(), env={k: v for k, v in os.environ.items() if k.startswith(WATCHED)})
        while True:
            text = self.inbox.get()
            if text is None:
                return 0
            self.init()
            self.run(text)


if __name__ == '__main__':
    sys.exit(Harness().main())
