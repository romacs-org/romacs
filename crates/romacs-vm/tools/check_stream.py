#!/usr/bin/env python3
"""Live request/reply and retained-state proof for the external form boundary."""
import argparse
import json
import selectors
import subprocess
from bench import ROOT, read_form
from eval_bench import gnu_stream

SOURCES = [
    "(setq agent-state 7)",
    "(defun agent-command (x &optional y &rest xs) (+ agent-state x (if y y 0) (length xs)))",
    "(agent-command 2 3 4 5)",
    "(setq agent-state 11)",
    "(agent-command 2)",
    "agent-state",
    "(defun gateway () (agent-command 2))",
    "(gateway)",
    "(defun agent-command (x &optional y &rest xs) (+ x 100))",
    "(agent-command 2)",
    "(gateway)",
    "(catch 'reply (funcall (lambda () (throw 'reply 42))))",
    "(condition-case e (signal 'error '(9)) (error (car (cdr e))))",
]


def exchange(command, payloads):
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    replies = []
    try:
        assert child.stdin is not None and child.stdout is not None
        with selectors.DefaultSelector() as selector:
            selector.register(child.stdout, selectors.EVENT_READ)
            for payload in payloads:
                child.stdin.write(payload + b'\n')
                child.stdin.flush()
                if not selector.select(timeout=20):
                    raise RuntimeError('reply not delivered before the next request/EOF')
                replies.append(json.loads(child.stdout.readline()))
        child.stdin.close()
        assert child.wait(timeout=20) == 0
        return replies
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', default=str(ROOT / 'target/release/romacs-vm'))
    args = parser.parse_args()
    payloads = [read_form(source) for source in SOURCES]
    oracle = exchange(gnu_stream(), payloads)
    actual = exchange([args.binary, '--eval-stream'], payloads)
    assert actual == oracle, (actual, oracle)
    invalid = exchange([args.binary, '--eval-stream'], [b'not JSON', b'["car",1]', b'["+",20,22]'])
    assert 'error' in invalid[0] and 'error' in invalid[1] and invalid[2] == {'value': '42'}
    output = dict(sources=SOURCES, transport=[json.loads(p) for p in payloads],
                  gnu=oracle, vm2=actual, recovery=invalid, passed=True,
                  boundary='one request, one flushed reply before next request; retained named functions and global state')
    (ROOT / 'evidence/s2-stream.json').write_text(json.dumps(output, indent=2) + '\n')
    print(f'{len(SOURCES)} live GNU request/reply comparisons; malformed/runtime-error recovery passed')

if __name__ == '__main__':
    main()
