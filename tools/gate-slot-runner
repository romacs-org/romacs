#!/usr/bin/env python3
"""Fair, lane-shared Neomacs heavy-job admission and scoped resource ownership."""
import argparse
import contextlib
import fcntl
import json
import os
from pathlib import Path
import re
import select
import shutil
import signal
import subprocess
import sys
import time

STATE = Path.home() / '.cache/neomacs-slot'
POLL_SECONDS = 2


def sync_directory():
    fd = os.open(STATE, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def slot_payload(sequence, unit, command):
    # The scope already exists when this guard runs. A delayed orphan launcher
    # may not run a payload for a pruned ticket that admission no longer counts.
    with transaction():
        try:
            ticket = json.loads((STATE / f'{int(sequence):020d}.json').read_text())
        except (OSError, ValueError):
            return 125
        if not ticket.get('active') or ticket.get('unit') != unit:
            return 125
        os.execvp(command[0], command)
    return 125


def log(message):
    print(f'neomacs-slot[{args.lane}]: {message}', file=sys.stderr, flush=True)


def systemctl(*argv):
    env = os.environ.copy()
    env.pop('NEOMACS_SLOT_INDEX', None)  # only this admission can grant a slot
    env.pop('LD_LIBRARY_PATH', None)
    try:
        return subprocess.run(['systemctl', '--user', *argv], env=env,
                              stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, timeout=5)
    except subprocess.TimeoutExpired:
        return subprocess.CompletedProcess(argv, 1, '', '')


def scope_alive(unit):
    result = systemctl('show', unit + '.scope', '--property=ActiveState', '--property=LoadState')
    fields = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
    # Unknown bus/query outcomes retain ownership, never prove scope death.
    if result.returncode != 0 or 'ActiveState' not in fields:
        return True
    return fields['ActiveState'] not in {'inactive', 'failed'}


def alive(ticket):
    # Only recorded scheduler PIDs are inspected; never signal them.
    try:
        os.kill(ticket['pid'], 0)
        fields = Path(f"/proc/{ticket['pid']}/stat").read_text().rsplit(') ', 1)[1].split()
        return fields[19] == ticket['start'] and fields[0] not in {'Z', 'X'}
    except (ProcessLookupError, FileNotFoundError):
        return False
    except PermissionError:
        return True
    except (KeyError, TypeError, ValueError, IndexError):
        return False


@contextlib.contextmanager
def transaction():
    with (STATE / 'queue.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield


def save(path, value):
    # Atomic, durable, owner-only journal.
    tmp = path.with_suffix('.tmp')
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, 'w') as stream:
            json.dump(value, stream)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(tmp, path)
        sync_directory()
    finally:
        tmp.unlink(missing_ok=True)


def append_event(event):
    # Never call under queue.lock. Completion records follow scope cleanup.
    # No ledger lock, buffering or retries: one bounded best-effort append.
    try:
        record = (json.dumps(event) + '\n').encode()
        if len(record) > select.PIPE_BUF:
            return
        fd = os.open(STATE / 'jobs.jsonl', os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_NONBLOCK, 0o600)
        try:
            os.write(fd, record)
        finally:
            with contextlib.suppress(OSError):
                os.close(fd)
    except (OSError, ValueError, TypeError):
        # Silent by design: a diagnostic write could block on a full stderr
        # pipe and withhold the job's rc. Missing records show in the report.
        pass


def records():
    result = []
    for path in sorted(STATE.glob('[0-9]*.json')):
        item = json.loads(path.read_text())
        if not alive(item) and not (item['active'] and scope_alive(item['unit'])):
            path.unlink()
        else:
            result.append((path, item))
    return result


def free_slot(capacity, running):
    used = {t.get('slot_index') for t in running}
    for index in range(capacity):
        if index in used:
            continue
        lease = Path.home() / '.cache/neomacs-slots' / str(index) / 'lease'
        lease.parent.mkdir(parents=True, exist_ok=True)
        with lease.open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                continue
            return index  # close releases this selection-only probe
    return None


def priority(ticket):
    # Live-editable: STATE/priorities.conf maps group -> int, read on every poll.
    if ticket.get('priority') is not None:
        return ticket['priority']
    try:
        table = json.loads((STATE / 'priorities.conf').read_text())
        # Exact group first, else the longest matching prefix (memgrowth -> memgrowth-*).
        keys = [k for k in table if ticket['group'].startswith(k)]
        return int(table[max(keys, key=len)]) if keys else 0
    except (OSError, ValueError, AttributeError):
        return 0


def rank(ticket):
    # Higher priority first; FIFO (creation time) within a priority.
    return (-priority(ticket), ticket['created'])


def interrupted(signum, _frame):
    raise KeyboardInterrupt


def main():
    global args
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('lane')
    parser.add_argument('--test-build', action='store_true', help='lower-memory test/check profiles; NEVER for release artifacts')
    parser.add_argument('--profile-build', '--full-debug', action='store_true', help='leave Cargo profiles untouched for timing/profiling/debug work')
    parser.add_argument('--light', action='store_true')
    parser.add_argument('--group')
    parser.add_argument('--mem', default='12G', help='MemoryMax (hard cap); no MemoryHigh throttle unless --mem-high')
    parser.add_argument('--reserve', help='memory reserved per running job for admission (default min(6G, MemoryMax))')
    parser.add_argument('--mem-high')
    parser.add_argument('--slots', type=int, default=4)  # 2026-10-15: 48 GB RAM, 4 heavy jobs x 4 workers = 16 cores (was 3)
    parser.add_argument('--priority', type=int, help='higher runs first; default from STATE/priorities.conf by group, else 0')
    parser.add_argument('--jobs', type=int)
    parser.add_argument('--test-threads', type=int)
    parser.add_argument('--max-min', type=int, default=240)
    argv = sys.argv[1:]
    try:
        split = argv.index('--')
    except ValueError:
        parser.error('expected -- CMD')
    args = parser.parse_args(argv[:split])
    cmd = argv[split + 1:]
    group = args.group or args.lane.split('-', 1)[0]
    if not cmd or not re.fullmatch(r'[A-Za-z0-9_][A-Za-z0-9_.-]*', group):
        parser.error('command and safe lane group are required')
    target_root = Path.home() / '.cache/neomacs-targets'
    jobs = args.jobs if args.jobs is not None else max(1, (os.cpu_count() or 1) // args.slots) if args.slots > 0 else 0
    threads = args.test_threads if args.test_threads is not None else jobs
    if min(args.slots, jobs, threads, args.max_min) < 1:
        parser.error('slots/jobs/test-threads/max-min must be positive')
    def memory(value):
        match = re.fullmatch(r'([1-9][0-9]*)([KMGT]?)', value)
        if not match:
            parser.error('memory must be positive integer bytes or K/M/G/T')
        return int(match[1]) * 1024 ** (' KMGT'.index(match[2]) if match[2] else 0)
    maximum = memory(args.mem)
    # MemoryHigh throttling (scopes cannot swap) shows up as app.slice memory
    # pressure and systemd-oomd then kills the whole build; cap hard only.
    high = memory(args.mem_high) if args.mem_high else None
    reserve = memory(args.reserve) if args.reserve else min(6 * 1024**3, maximum)
    if high is not None and high > maximum:
        parser.error('MemoryHigh must not exceed MemoryMax')
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    if args.light:
        env = os.environ.copy()
        env.setdefault('ROMACS_TEST_FONTS_DIR', str(Path.home() / '.cache/romacs-test-fonts'))
        env.pop('NEOMACS_SLOT_INDEX', None)
        pooled = Path.home() / '.cache/neomacs-slots'
        if Path(env.get('CARGO_TARGET_DIR', '.')).resolve().is_relative_to(pooled.resolve()):
            env.pop('CARGO_TARGET_DIR', None)
    cache = Path.home() / '.cache/sccache'
    sccache = shutil.which('sccache') or str(Path.home() / '.nix-profile/bin/sccache')
    if not args.light and not os.access(sccache, os.X_OK):
        parser.error('sccache missing; install with Nix profile before heavy builds')
    target = target_root / group
    light_env = env if args.light else None
    env = os.environ.copy()
    env.pop('NEOMACS_SLOT_INDEX', None)  # only this admission can grant a slot
    env.setdefault('ROMACS_TEST_FONTS_DIR', str(Path.home() / '.cache/romacs-test-fonts'))
    env.update(CARGO_TARGET_DIR=str(target), CARGO_BUILD_JOBS=str(jobs),
               NEXTEST_TEST_THREADS=str(threads), RUST_TEST_THREADS=str(threads), RUSTC_WRAPPER=sccache, SCCACHE_CLIENT_SIDE='1',
               SCCACHE_DIR=str(cache), SCCACHE_CACHE_SIZE='20G')
    # Client-side compilation keeps rustc inside its build's capped scope.
    env.pop('SCCACHE_ERROR_LOG', None)
    env.pop('SCCACHE_LOG', None)  # logging disables client-side mode in sccache 0.17
    env['SCCACHE_CONF'] = '/dev/null'  # do not inherit distributed-compilation configuration
    protected = {'perf', 'gc', 'redisplay', 'gpu', 'io', 'latency'}
    full_profile = args.profile_build or bool(protected.intersection(args.lane.split('-'))) or group in protected
    correctness = args.test_build or (Path(cmd[0]).name == 'cargo' and len(cmd) > 1 and cmd[1] in {'test', 'check', 'nextest'})
    if correctness and not full_profile:
        env.update(CARGO_PROFILE_RELEASE_LTO='off', CARGO_PROFILE_RELEASE_CODEGEN_UNITS='16')
    # Server workers must originate in the scheduler, not a capped build scope.
    # Start sccache only after winning admission, before launching the scope.
    if args.light:
        env = light_env
    STATE.mkdir(parents=True, exist_ok=True)
    unit = f'neomacs-slot-{os.getpid()}-{time.time_ns()}'
    ticket = dict(pid=os.getpid(), start=Path('/proc/self/stat').read_text().split(') ', 1)[1].split()[19],
                  group=group, lane=args.lane, group_lock=not args.light, kind='light' if args.light else 'heavy',
                  active=False, slot_index=None, unit=unit, created=time.time(), slots=args.slots,
                  maximum=maximum, high=reserve, priority=args.priority)
    with transaction():
        seq_path = STATE / 'sequence'
        seq = int(seq_path.read_text()) + 1 if seq_path.exists() else 1
        seq_path.write_text(str(seq))
        path = STATE / f'{seq:020d}.json'
        save(path, ticket)
    admitted = False
    started = None
    admitted_at = None
    rc = None
    recorded_priority = priority(ticket)
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    next_notice = 0.0
    wait_cause = wait_blocker = wait_detail = None
    try:
        while not admitted:
            with transaction():
                all_records = records()
                running = [t for _, t in all_records if t['active'] and t.get('kind') != 'light']
                ticket_seqs = {id(t): int(p.stem) for p, t in all_records if p is not None}
                wall_now = time.time()
                def blocker(t):
                    seq = ticket_seqs.get(id(t))
                    return seq, f'blocker=#{seq if seq is not None else "unknown"} {t.get("lane", t["group"])} {max(0, int((wall_now - t["created"]) / 60))}m'
                waiting = [t for _, t in all_records if not t['active'] and t.get('kind') != 'light']
                eligible = sorted(waiting, key=rank)
                capacity = min([args.slots] + [t['slots'] for _, t in all_records if t.get('kind') != 'light'])
                slot_index = free_slot(capacity, running)
                available = int(next(line.split()[1] for line in Path('/proc/meminfo').read_text().splitlines() if line.startswith('MemAvailable:'))) * 1024
                reserved = 0
                for t in running:
                    cgroup = systemctl('show', t['unit'] + '.scope', '--property=ControlGroup', '--value').stdout.strip()
                    current = Path('/sys/fs/cgroup' + cgroup) / 'memory.current'
                    used = int(current.read_text()) if cgroup and current.exists() else 0
                    reserved += max(0, t.get('high', t['maximum']) - used)
                # STATE/reserved.conf maps a group prefix -> slots held open for it:
                # other groups may not take a slot a reserved prefix is not using.
                try:
                    held_for = json.loads((STATE / 'reserved.conf').read_text())
                except (OSError, ValueError):
                    held_for = {}
                def blocking_reasons(waiter):
                    mine = any(waiter['group'].startswith(p) for p in held_for)
                    held = 0 if mine else sum(
                        max(0, n - sum(1 for t in running if t['group'].startswith(p)))
                        for p, n in held_for.items())
                    reasons = []
                    if waiter.get('group_lock', True):
                        busy = next((t for t in running if t['group'] == waiter['group'] and t.get('group_lock', True)), None)
                        if busy is not None:
                            seq, detail = blocker(busy)
                            reasons.append(('group-busy', seq, detail))
                    if len(running) >= capacity:
                        seq, detail = blocker(running[0]) if running else (None, '')
                        reasons.append(('capacity', seq, f'{detail} running={len(running)}/{capacity}'))
                    elif len(running) >= capacity - held:
                        reasons.append(('reservation', None, f'prefix={",".join(held_for)} held={held} running={len(running)}/{capacity}'))
                    if slot_index is None and len(running) < capacity:
                        reasons.append(('slot-busy', None, 'all slot leases held'))
                    need = waiter.get("high", waiter["maximum"]) + 3 * 1024**3  # 3 GiB desktop headroom over running reservations (48 GB host)
                    if available - reserved < need:
                        reasons.append(('memory', None, f'need={need} avail={available - reserved}'))
                    return reasons

                # All ranking, resource checks and the active write share queue.lock.
                # A group-only block cannot benefit from a host-wide aging fence.
                winner = None
                fence = None
                fence_reasons = []
                for waiter in eligible:
                    reasons = blocking_reasons(waiter)
                    if not reasons:
                        winner = waiter
                        break
                    if wall_now - waiter['created'] >= 30 * 60 and [r[0] for r in reasons] != ['group-busy']:
                        fence, fence_reasons = waiter, reasons
                        break
                first = args.light or (winner is not None and winner['pid'] == os.getpid())
                if first:
                    # The ticket reserves admission; the payload takes the kernel lease.
                    ticket['slot_index'] = None if args.light else slot_index
                    admitted = ticket['active'] = True
                    admitted_at = time.time()
                    recorded_priority = priority(ticket)
                    save(path, ticket)
                position = next((i + 1 for i, t in enumerate(eligible) if t['pid'] == os.getpid()), 0)
                if not admitted:
                    reasons = blocking_reasons(ticket)
                    if fence is not None and eligible.index(fence) < position - 1:
                        blocker_seq, detail = blocker(fence)
                        reasons.insert(0, ('fenced-by', blocker_seq, detail + ' (' + '; '.join(f'{c} {d}' for c, _, d in fence_reasons) + ')'))
                    elif not reasons and winner is not None:
                        blocker_seq, detail = blocker(winner)
                        reasons.append(('rank', blocker_seq, detail))
                    wait_cause, wait_blocker, _ = reasons[0]
                    wait_detail = '; '.join(f'cause={c} {d}' for c, _, d in reasons)
            if not admitted:
                if time.monotonic() >= next_notice:
                    log(f'queued position={position} running={len(running)} group={group}; {wait_detail}; ETA unknown')
                    next_notice = time.monotonic() + 300
                time.sleep(POLL_SECONDS)
        if not args.light and not full_profile:
            index = ticket['slot_index']
            target = Path.home() / '.cache/neomacs-slots' / str(index) / 'target'
            env.update(NEOMACS_SLOT_INDEX=str(index), CARGO_TARGET_DIR=str(target))
        if not args.light:
            subprocess.run([sccache, '--start-server'], env=env, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, check=False)
        lease = Path.home() / '.cache/neomacs-slots' / str(ticket['slot_index']) / 'lease'
        # A delayed orphan launcher must win the lease again, never share a slot.
        leased_cmd = ['sh', '-c',
                      'lease=$1; index=$2; shift 2; '
                      'flock --nonblock --conflict-exit-code 75 "$lease" "$@"; rc=$?; '
                      'if [ "$rc" -eq 75 ]; then echo "slot $index busy" >&2; fi; exit "$rc"',
                      'slot-lease', str(lease), str(ticket['slot_index']), *cmd]
        if args.light:
            leased_cmd = cmd
        else:
            target.mkdir(parents=True, exist_ok=True)
        started = time.monotonic()
        log(f'admitted ticket={seq} group={group} slots={args.slots} jobs={jobs} threads={threads} target={target} unit={unit} MemoryHigh={high or "infinity"} MemoryMax={maximum} reserve={reserve}')
        scope_env = env.copy()
        scope_env.pop('LD_LIBRARY_PATH', None)
        payload_env = ['env']
        if 'LD_LIBRARY_PATH' in env:
            payload_env.append('LD_LIBRARY_PATH=' + env['LD_LIBRARY_PATH'])
        rc = subprocess.call(['systemd-run', '--user', '--scope', '--quiet', '--slice=neomacs-builds.slice', '--unit=' + unit,
                              '-p', 'CPUWeight=20',
                              *([] if args.light else ['-p', f'MemoryHigh={high if high is not None else "infinity"}',
                                                       '-p', f'MemoryMax={maximum}', '-p', 'MemorySwapMax=0']),
                              *payload_env, 'choom', '-n', '500', '--', 'timeout', '--kill-after=60',
                              f'{args.max_min}m', 'nice', '-n', '10',
                              sys.executable, str(Path(__file__).resolve()), '--slot-payload', str(seq), unit,
                              *leased_cmd], env=scope_env)
        rc = rc if rc >= 0 else 128 - rc
        return rc
    except KeyboardInterrupt:
        rc = 130
        return rc
    finally:
        # Keep admission until our exact owned scope is gone.
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        if admitted:
            systemctl('stop', unit + '.scope')
            while scope_alive(unit):
                time.sleep(1)
        with transaction():
            path.unlink(missing_ok=True)
            if started is not None:
                duration_file = STATE / 'duration'
                old = float(duration_file.read_text()) if duration_file.exists() else 600
                duration_file.write_text(str(old * .75 + (time.monotonic() - started) * .25))
        if started is not None:
            log(f'done unit={unit} wall={time.monotonic() - started:.2f}s')
        append_event(dict(seq=seq, unit=unit, lane=args.lane, group=group, priority=recorded_priority,
                          created=ticket['created'], admitted=admitted_at, finished=time.time(),
                          rc=rc, mode=cmd[:3], kind=ticket['kind'],
                          group_lock=ticket['group_lock'], slot_index=ticket['slot_index'],
                          wait_cause=wait_cause, wait_blocker=wait_blocker, wait_detail=wait_detail))


if __name__ == '__main__':
    if len(sys.argv) >= 5 and sys.argv[1] == '--slot-payload':
        sys.exit(slot_payload(sys.argv[2], sys.argv[3], sys.argv[4:]))
    sys.exit(main())
