#!/usr/bin/env python3
"""Runner-local priority/FIFO queue, sharing the legacy benchmark lock.

Tickets live in ~/romacs-bench/queue. Live waiting tickets heartbeat; dead owners
are reaped on the same host. A stable sequence lock orders FIFO ties. Each ticket
holds the legacy lock at most 45 minutes, including its isolated child process.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shlex
import signal
import socket
import subprocess
import sys
import time

MAX_HOLD = 45 * 60


def priority(lane):
    # ROADMAP M1 order: memory, W22, SQLite, VM P1, W17, then W4/P2/P3.
    name = lane.lower()
    for rank, names in enumerate((('mem',), ('w22',), ('sqlite',), ('evp1', 'vm-p1'),
                                 ('w17',), ('w4', 'evp2', 'evp3'))):
        if any(name.startswith(prefix) for prefix in names):
            return 10 + rank
    return 50


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def tickets(root):
    rows = []
    for path in (root / 'queue').glob('*.json'):
        ticket = json.loads(path.read_text())
        if not alive(ticket['pid']):
            path.unlink(missing_ok=True)
            continue
        rows.append(dict(ticket, path=str(path)))
    return sorted(rows, key=lambda t: (t['priority'], t['sequence']))


def publish(path, ticket):
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(ticket, sort_keys=True) + '\n')
    temporary.replace(path)


def run_ticket(root, label, rank, expected_seconds, command, *, cwd=None, env=None,
               stdout=None, stderr=None):
    if not 0 < expected_seconds <= MAX_HOLD:
        raise ValueError('expected duration must be 1..2700 seconds; split/requeue longer work')
    root = Path(root).resolve()
    queue = root / 'queue'
    queue.mkdir(parents=True, exist_ok=True)
    with (queue / '.sequence.lock').open('a') as sequence_lock:
        fcntl.flock(sequence_lock, fcntl.LOCK_EX)
        seq_path = queue / '.sequence'
        sequence = int(seq_path.read_text()) + 1 if seq_path.exists() else 1
        seq_path.write_text(str(sequence))
        path = queue / f'{sequence:020d}.json'
        ticket = {'label': label, 'priority': rank, 'sequence': sequence, 'pid': os.getpid(),
                  'host': socket.gethostname(), 'expected_seconds': expected_seconds,
                  'created': time.time(), 'heartbeat': time.time(), 'status': 'waiting'}
        publish(path, ticket)
    try:
        with (root / '.lock').open('a') as lock:
            while True:
                ticket['heartbeat'] = time.time()
                publish(path, ticket)
                ordered = tickets(root)
                if ordered and ordered[0]['sequence'] == sequence and not (root / 'PAUSE').exists():
                    try:
                        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    except BlockingIOError:
                        pass
                    else:
                        # A higher priority ticket may have arrived while legacy
                        # lock acquisition was pending. Recheck before admission.
                        ordered = tickets(root)
                        if ordered and ordered[0]['sequence'] == sequence:
                            break
                        fcntl.flock(lock, fcntl.LOCK_UN)
                time.sleep(1)
            ticket.update(status='running', started=time.time())
            publish(path, ticket)
            supervised = [sys.executable, str(Path(__file__).resolve()), '--supervise', str(MAX_HOLD), *command]
            child_env = dict(os.environ if env is None else env, ROMACS_BENCH_LOCK_FD=str(lock.fileno()))
            process = subprocess.Popen(supervised, cwd=cwd, env=child_env, stdout=stdout,
                                       stderr=stderr, start_new_session=True,
                                       pass_fds=(lock.fileno(),))
            try:
                return process.wait(timeout=MAX_HOLD)
            except BaseException:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                raise
    finally:
        path.unlink(missing_ok=True)


def list_remote(host):
    worker = Path(__file__).resolve()
    command = 'python3 -c ' + shlex.quote(worker.read_text()) + ' --list'
    code = subprocess.run(['ssh', host, command], check=False).returncode
    return 0 if code == 0 else 2


def main():
    if len(sys.argv) > 3 and sys.argv[1] == '--supervise':
        # This process keeps the legacy lock and an independent hard deadline
        # even if its SSH client or queue owner is SIGKILLed.
        def expire(signum, frame):
            os.killpg(os.getpgrp(), signal.SIGKILL)
        signal.signal(signal.SIGALRM, expire)
        signal.setitimer(signal.ITIMER_REAL, float(sys.argv[2]))
        descriptor = int(os.environ['ROMACS_BENCH_LOCK_FD'])
        return subprocess.run(sys.argv[3:], check=False, pass_fds=(descriptor,)).returncode
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--list', action='store_true')
    parser.add_argument('--root', type=Path, default=Path.home() / 'romacs-bench')
    args = parser.parse_args()
    print(json.dumps(tickets(args.root), indent=2))
    return 0


if __name__ == '__main__':
    sys.exit(main())
