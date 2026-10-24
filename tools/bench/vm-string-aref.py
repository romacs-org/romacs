"""Guard ASCII aref against a matched-profile baseline (whole-process PMU).

Usage: python3 tools/bench/vm-string-aref.py BASE_BINARY CANDIDATE_BINARY
Five interleaved rounds; same source as the original string-substrate lane.
GC is disabled by the 1GB floor, verified in each runtime's heap receipt.
This guards instructions only, not wall-clock acceptance.
"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('counter', ROOT / 'scripts/romacs_accept_supervisor.py')
assert spec is not None and spec.loader is not None
counter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(counter)
source = json.loads(Path(__file__).with_name('vm-string-programs.json').read_text())['aref']


def run(binary):
    with tempfile.TemporaryFile() as inp, tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        inp.write(source.encode())
        inp.seek(0)
        pid = os.fork()
        if pid == 0:
            try:
                os.dup2(inp.fileno(), 0)
                os.dup2(out.fileno(), 1)
                os.dup2(err.fileno(), 2)
                os.kill(os.getpid(), signal.SIGSTOP)
                os.execv(binary, [binary, '--source', '--heap-stats', '--gc-floor', '1000000000'])
            finally:
                os._exit(127)
        event = None
        reaped = False
        try:
            os.waitpid(pid, os.WUNTRACED)
            event = counter._Counter(pid)
            event.enable()
            os.kill(pid, signal.SIGCONT)
            deadline = time.monotonic() + 60
            while True:
                done, status = os.waitpid(pid, os.WNOHANG)
                if done:
                    reaped = True
                    break
                if time.monotonic() > deadline:
                    raise RuntimeError('workload timeout')
                time.sleep(.001)
            event.disable()
            raw = event.read()
            count = counter._counter_delta((0, 0, 0), raw)
            assert raw[1] == raw[2], raw
            out.seek(0)
            err.seek(0)
            stdout, stderr = out.read().decode(), err.read().decode()
            assert status == 0 and stdout.strip() == '97', (status, stdout, stderr)
            stats = json.loads(stderr)
            assert stats['collections_during_run'] == 0, stats
            return dict(count=count, raw=raw, stdout=stdout, stats=stats, load=os.getloadavg())
        finally:
            if event is not None:
                event.close()
            if not reaped:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)


def main():
    base, after = [str(Path(p).resolve(strict=True)) for p in sys.argv[1:]]
    cpu = min(os.sched_getaffinity(0))
    os.sched_setaffinity(0, {cpu})
    print(json.dumps(dict(cpu=cpu, source_sha256=hashlib.sha256(source.encode()).hexdigest(),
                          binaries={p: hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in (base, after)})), flush=True)
    results = {'base': [], 'after': []}
    for i in range(5):
        arms = [('base', base), ('after', after)]
        for side, binary in arms if i % 2 == 0 else reversed(arms):
            sample = run(binary)
            results[side].append(sample)
            print(json.dumps(dict(side=side, sample=sample)), flush=True)
    baseline = min(r['count'] for r in results['base'])
    candidate = max(r['count'] for r in results['after'])
    print(json.dumps(dict(base_min=baseline, after_max=candidate, ratio=candidate / baseline)), flush=True)
    assert candidate * 100 <= baseline * 102, f'ASCII aref instructions {candidate / baseline:.6f} exceed 1.02'


if __name__ == '__main__':
    main()
