#!/usr/bin/env python3
"""A persistent JSON-RPC harness for agent session tests (agent_live.py
`rpc'), in the shape of ACP's agents: newline-delimited JSON-RPC 2.0 on
stdio, `initialize' then `session/new' (or `session/load' of an earlier
session) before any `session/prompt'.  Its driver is defined by the test
alone (harness/fakerpc.el), so it proves a harness is added by its driver
and nothing else.  Its "model" is a script, one command per prompt:

  say TEXT     reply TEXT, word by word (session/update notifications)
  ask COMMAND  ask permission for Bash COMMAND (session/request_permission,
               a request to the client) and reply with the answer

It writes each session's transcript to $FAKERPC_HOME (default ~/.fakerpc)
sessions/ID.jsonl, and logs every request it reads to $FAKE_HARNESS_LOG.
$FAKERPC_SLOW_INIT delays its initialize answer by that many seconds;
from its start it writes unsolicited notifications for a second.
"""
import json
import os
import sys
import threading
import time
import uuid

OUT = threading.Lock()
LOG = os.environ.get('FAKE_HARNESS_LOG')
HOME = os.environ.get('FAKERPC_HOME') or os.path.expanduser('~/.fakerpc')


def emit(obj):
    with OUT:
        sys.stdout.write(json.dumps(obj) + '\n')
        sys.stdout.flush()


def log(**fields):
    if LOG:
        with open(LOG, 'a') as handle:
            handle.write(json.dumps({'t': time.time(), 'pid': os.getpid(), **fields}) + '\n')


def transcript(session, **record):
    path = os.path.join(HOME, 'sessions', session + '.jsonl')
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'a') as handle:
        handle.write(json.dumps(record, separators=(',', ':')) + '\n')


def chatter():
    """Unsolicited notifications from the start: some land between a
    startup request's write and its echo in the journal."""
    for i in range(50):
        emit({'jsonrpc': '2.0', 'method': 'notice', 'params': {'n': i}})
        time.sleep(0.02)


def main():
    log(rpc_started=True, argv=sys.argv[1:], home=os.environ.get('FAKERPC_HOME'), cwd=os.getcwd())
    threading.Thread(target=chatter, daemon=True).start()
    initialized = False
    session = None
    answers = {}
    waiting = threading.Event()
    for line in sys.stdin:
        try:
            message = json.loads(line)
        except ValueError:
            continue
        method, rid, params = message.get('method'), message.get('id'), message.get('params') or {}
        if method is None:
            # The client's answer to our permission request.
            answers[rid] = message.get('result')
            log(rpc='answer', id=rid, result=message.get('result'))
            waiting.set()
            continue
        log(rpc=method, id=rid, session=params.get('sessionId'), initialized=initialized, current=session)
        if method == 'initialize':
            time.sleep(float(os.environ.get('FAKERPC_SLOW_INIT') or 0))
            initialized = True
            emit({'jsonrpc': '2.0', 'id': rid, 'result': {'protocolVersion': 1, 'agent': 'fakerpc'}})
        elif not initialized:
            emit({'jsonrpc': '2.0', 'id': rid, 'error': {'code': -32002, 'message': 'not initialized'}})
        elif method == 'session/new':
            session = 'rpc-' + uuid.uuid4().hex[:12]
            emit({'jsonrpc': '2.0', 'id': rid, 'result': {'sessionId': session}})
        elif method == 'session/load':
            session = params.get('sessionId')
            emit({'jsonrpc': '2.0', 'id': rid, 'result': {'sessionId': session}})
        elif method == 'session/cancel':
            pass
        elif method == 'session/prompt':
            if session is None or params.get('sessionId') != session:
                emit({'jsonrpc': '2.0', 'id': rid, 'error': {'code': -32003, 'message': 'no session'}})
                continue
            text = params.get('prompt') or ''
            transcript(session, type='prompt', text=text, cwd=os.getcwd())
            words = text.split(' ', 1)
            reply = ''
            if words[0] == 'say':
                reply = words[1] if len(words) > 1 else ''
            elif words[0] == 'ask':
                ask = 'perm-%s' % uuid.uuid4().hex[:6]
                waiting.clear()
                emit({'jsonrpc': '2.0', 'id': ask, 'method': 'session/request_permission',
                      'params': {'sessionId': session, 'tool': 'Bash', 'input': {'command': words[1]}}})
                # Answers come on stdin: read them here, in order.
                for answer_line in sys.stdin:
                    answer = json.loads(answer_line)
                    if answer.get('id') == ask and 'method' not in answer:
                        answers[ask] = answer.get('result')
                        log(rpc='answer', id=ask, result=answer.get('result'))
                        break
                reply = 'permission %s' % ((answers.get(ask) or {}).get('outcome'))
            else:
                reply = 'unknown command'
            for word in reply.split(' '):
                emit({'jsonrpc': '2.0', 'method': 'session/update',
                      'params': {'sessionId': session, 'text': word + ' '}})
            transcript(session, type='reply', text=reply)
            emit({'jsonrpc': '2.0', 'id': rid, 'result': {'stopReason': 'end_turn', 'text': reply}})
        else:
            emit({'jsonrpc': '2.0', 'id': rid, 'error': {'code': -32601, 'message': 'no method ' + str(method)}})


if __name__ == '__main__':
    main()
