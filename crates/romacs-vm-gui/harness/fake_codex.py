#!/usr/bin/env python3
"""A stand-in for `codex exec --json' (one turn per run), for agent
session tests (agent_live.py).  It reads the prompt from stdin (`-'),
writes the JSONL events Codex writes, and exits.  `codex exec resume ID'
continues thread ID.  Its "model" is a script on the prompt's first line:

  say TEXT      reply TEXT
  slow N        sleep N seconds, then reply `slow done' (a long turn)
  fail          end the turn with turn.failed
  anything else reply `VERDICT: APPROVE' and the prompt's line count

Every run is logged to $FAKE_HARNESS_LOG as one JSON line (argv,
CODEX_HOME, the thread, the prompt's first line).
"""
import json
import os
import sys
import time
import uuid

LOG = os.environ.get('FAKE_HARNESS_LOG')


def emit(obj):
    sys.stdout.write(json.dumps(obj) + '\n')
    sys.stdout.flush()


def main():
    argv = sys.argv[1:]
    thread = argv[argv.index('resume') + 1] if 'resume' in argv else str(uuid.uuid4())
    prompt = sys.stdin.read() if argv and argv[-1] == '-' else argv[-1]
    first = prompt.strip().splitlines()[0] if prompt.strip() else ''
    if LOG:
        with open(LOG, 'a') as handle:
            handle.write(json.dumps({'t': time.time(), 'codex': thread, 'argv': argv, 'pid': os.getpid(),
                                     'codex_home': os.environ.get('CODEX_HOME'), 'prompt': first[:200]}) + '\n')
    emit({'type': 'thread.started', 'thread_id': thread})
    emit({'type': 'turn.started'})
    if first == 'fail':
        emit({'type': 'turn.failed', 'error': {'message': 'the fake model failed'}})
        return 1
    if first.startswith('slow '):
        time.sleep(float(first.split()[1]))
        reply = 'slow done'
    elif first.startswith('say '):
        reply = first[4:]
    else:
        reply = f'VERDICT: APPROVE\nreviewed {len(prompt.splitlines())} lines'
    emit({'type': 'item.completed', 'item': {'id': 'item_0', 'type': 'agent_message', 'text': reply}})
    emit({'type': 'turn.completed', 'usage': {'input_tokens': 10, 'cached_input_tokens': 0, 'output_tokens': 3}})
    return 0


if __name__ == '__main__':
    sys.exit(main())
