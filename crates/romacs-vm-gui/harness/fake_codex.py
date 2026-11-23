#!/usr/bin/env python3
"""A stand-in for `codex app-server' (codex-cli 0.164), for agent session
tests (agent_live.py): JSON-RPC lines on stdio, `initialize' then
`thread/start' or `thread/resume', a `turn/start' per message, `turn/steer'
into the turn running and `turn/interrupt' ending it; `model/list' answers
MODELS, as codex-cli 0.171 does.  Its "model" is a
script on the prompt's first line:

  say TEXT      reply TEXT
  slow N        sleep N seconds, then reply `slow done' (a long turn; a
                steer's text ends it early, `steered: TEXT')
  wait FILE     a turn held until FILE exists (60 s at most), then reply
                `released' (an interrupt ends it)
  fail          end the turn failed
  Evaluate ...  Gnosis's evaluator prompt: answered as fake_harness.py does
  anything else reply `VERDICT: APPROVE' and the prompt's line count

Every turn is logged to $FAKE_HARNESS_LOG as one JSON line (the thread,
whether it was resumed, the turn's model and effort, CODEX_HOME, the
prompt's first line, the process's argv and pid).
A spent file in CODEX_HOME makes accepted turns end at the usage limit.
A refuse file makes turn/start return a JSON-RPC error before acceptance.
"""
import json
import os
import sys
import threading
import time
import uuid

from fake_harness import message_body

LOG = os.environ.get('FAKE_HARNESS_LOG')
OUT = threading.Lock()
# What `model/list' answers.
MODELS = [
    {'id': 'gpt-fake-sol', 'model': 'gpt-fake-sol', 'displayName': 'GPT-Fake-Sol', 'description': '',
     'hidden': False, 'isDefault': True, 'defaultReasoningEffort': 'low',
     'supportedReasoningEfforts': [{'reasoningEffort': e, 'description': ''} for e in ('low', 'medium', 'high')]},
    {'id': 'gpt-6-luna', 'model': 'gpt-6-luna', 'displayName': 'GPT-6-Luna', 'description': '',
     'hidden': False, 'isDefault': False, 'defaultReasoningEffort': 'medium',
     'supportedReasoningEfforts': [{'reasoningEffort': e, 'description': ''} for e in ('low', 'medium', 'high')]},
]


def emit(obj):
    with OUT:
        sys.stdout.write(json.dumps(obj) + '\n')
        sys.stdout.flush()


def notify(method, **params):
    emit({'method': method, 'params': params})


class Thread:
    def __init__(self, thread_id, resumed):
        self.id, self.resumed = thread_id, resumed
        self.turn, self.steers, self.stop = None, [], threading.Event()


def reply_for(prompt, first):
    if prompt.startswith('Evaluate the learner response'):
        from fake_harness import evaluator_reply
        return evaluator_reply(prompt)
    if first.startswith('say '):
        return first[4:]
    return f'VERDICT: APPROVE\nreviewed {len(prompt.splitlines())} lines'


def script_line(prompt):
    """The prompt's script: its first line, after a delegated task's role
    preamble (`Act as the ROLE sub-agent for this task.')."""
    lines = [l for l in message_body(prompt).strip().splitlines() if l.strip()]
    if lines and lines[0].startswith('Act as the ') and lines[0].endswith(' sub-agent for this task.'):
        lines = lines[1:]
    return lines[0] if lines else ''


def turn_outcome(thread, turn, prompt, first):
    if first == 'fail':
        return 'failed', {'message': 'the fake model failed'}, None
    if first.startswith('wait '):
        path, deadline = first.split(' ', 1)[1], time.monotonic() + 60
        while not os.path.exists(path) and time.monotonic() < deadline and thread.turn == turn:
            thread.stop.wait(.05)
        return ('completed', None, 'released') if thread.turn == turn else ('interrupted', None, None)
    if first.startswith('slow '):
        thread.stop.wait(float(first.split()[1]))
        if thread.turn != turn:
            return 'interrupted', None, None
        return 'completed', None, 'steered: ' + thread.steers[-1] if thread.steers else 'slow done'
    return 'completed', None, reply_for(prompt, first)


def account_marker(name):
    home = os.environ.get('CODEX_HOME')
    return home and os.path.isfile(os.path.join(home, name))


def run_turn(thread, turn, prompt):
    first = script_line(prompt)
    notify('turn/started', threadId=thread.id, turn={'id': turn, 'status': 'inProgress'})
    status, error, reply = turn_outcome(thread, turn, prompt, first)
    if account_marker('spent'):
        notify('account/rateLimits/updated', rateLimits={'primary': {'usedPercent': 100, 'windowDurationMins': 300,
                                                                     'resetsAt': int(time.time()) + 3600}})
        status, error, reply = 'failed', {'message': 'Usage limit reached', 'codexErrorInfo': 'usageLimitExceeded'}, None
    if status == 'completed':
        log_input(thread, seen=prompt)
    if reply is not None:
        item = f'msg_{uuid.uuid4().hex}'
        notify('item/started', threadId=thread.id, turnId=turn,
               item={'type': 'agentMessage', 'id': item, 'text': ''})
        notify('item/agentMessage/delta', threadId=thread.id, turnId=turn, itemId=item, delta=reply)
        notify('item/completed', threadId=thread.id, turnId=turn,
               item={'type': 'agentMessage', 'id': item, 'text': reply})
    notify('thread/tokenUsage/updated', threadId=thread.id, turnId=turn,
           tokenUsage={'last': {'totalTokens': 13}, 'total': {'totalTokens': 13}, 'modelContextWindow': 1000})
    thread.turn, thread.steers = None, []
    thread.stop.clear()
    notify('turn/completed', threadId=thread.id, turn={'id': turn, 'status': status, 'error': error})


def text_of(params):
    return ''.join(item.get('text', '') for item in params.get('input', []) if item.get('type') == 'text')


def log_input(thread, **fields):
    if LOG:
        with open(LOG, 'a') as handle:
            handle.write(json.dumps({'t': time.time(), 'pid': os.getpid(), 'session': thread.id,
                                     'config_dir': os.environ.get('CODEX_HOME'), **fields}) + '\n')


def log_turn(thread, params, prompt, refused):
    if LOG:
        first = script_line(prompt)
        fields = {'attempted': prompt} if refused else {'received': prompt}
        with open(LOG, 'a') as handle:
            handle.write(json.dumps({'t': time.time(), 'codex': thread.id, 'resumed': thread.resumed,
                                     'model': params.get('model'), 'effort': params.get('effort'),
                                     'argv': sys.argv[1:], 'pid': os.getpid(),
                                     'codex_home': os.environ.get('CODEX_HOME'), 'prompt': first[:200],
                                     'last': (prompt.strip().splitlines() or [''])[-1][:200],
                                     'session': thread.id, 'config_dir': os.environ.get('CODEX_HOME'), **fields,
                                     'env': {k: v for k, v in os.environ.items()
                                             if k.startswith(('ROMACS_', 'CLAUDE', 'CODEX_', 'T3CODE_', 'AGENT_TEST_'))}}) + '\n')


def start_turn(thread, rid, params):
    prompt = text_of(params)
    refused = account_marker('refuse')
    log_turn(thread, params, prompt, refused)
    if refused:
        emit({'id': rid, 'error': {'code': -32000, 'message': 'turn/start refused'}})
        return
    turn = str(uuid.uuid4())
    thread.turn = turn
    emit({'id': rid, 'result': {'turn': {'id': turn, 'status': 'inProgress'}}})
    threading.Thread(target=run_turn, args=(thread, turn, prompt), daemon=True).start()


def main():
    thread = None
    for line in sys.stdin:
        message = json.loads(line)
        method, rid, params = message.get('method'), message.get('id'), message.get('params') or {}
        if method == 'initialize':
            emit({'id': rid, 'result': {'userAgent': 'fake_codex'}})
        elif method == 'model/list':
            emit({'id': rid, 'result': {'data': MODELS, 'nextCursor': None}})
        elif method in ('thread/start', 'thread/resume'):
            thread = Thread(params.get('threadId') or str(uuid.uuid4()), method == 'thread/resume')
            emit({'id': rid, 'result': {'thread': {'id': thread.id}, 'model': params.get('model') or 'fake'}})
        elif method == 'turn/start':
            start_turn(thread, rid, params)
        elif method == 'turn/steer':
            if thread.turn and params.get('expectedTurnId') == thread.turn:
                thread.steers.append(text_of(params))
                thread.stop.set()
                emit({'id': rid, 'result': {'turnId': thread.turn}})
            else:
                emit({'id': rid, 'error': {'code': -32600, 'message': 'no active turn to steer'}})
        elif method == 'turn/interrupt':
            if thread.turn == params.get('turnId'):
                thread.turn = None
                thread.stop.set()
            emit({'id': rid, 'result': {}})
        elif rid is not None and method:
            emit({'id': rid, 'error': {'code': -32601, 'message': f'fake_codex does not serve {method}'}})
    return 0


if __name__ == '__main__':
    sys.exit(main())
