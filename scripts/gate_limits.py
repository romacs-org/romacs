"""Remove a poisoned inherited RT watchdog before entering the gate's scope.

Linux can kill SCHED_OTHER threads with per-thread CPU timers when a desktop
session passes down RLIMIT_RTTIME=0/0. A scope inherits that hard limit; a
transient user service can start without it, without sudo or session changes.
"""
import json
import os
import pathlib
import resource
import signal
import stat
import subprocess
import sys
import tempfile
import uuid


def restore(path):
    """Restore the caller's context, changing only the zero RT watchdog."""
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(fd) as stream:
        info = os.fstat(stream.fileno())
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
                or stat.S_IMODE(info.st_mode) != 0o600):
            raise ValueError('unsafe resource handoff')
        state = json.load(stream)
    os.unlink(path)
    unlimited = (resource.RLIM_INFINITY, resource.RLIM_INFINITY)
    if resource.getrlimit(resource.RLIMIT_RTTIME) != unlimited:
        raise ValueError('user service did not clear RLIMIT_RTTIME')
    for number, limits in state['limits']:
        resource.setrlimit(number, tuple(limits))
        if resource.getrlimit(number) != tuple(limits):
            raise ValueError('user service did not preserve resource limits')
    os.sched_setaffinity(0, state['affinity'])
    os.setpriority(os.PRIO_PROCESS, 0, state['nice'])
    os.umask(state['umask'])
    os.chdir(state['cwd'])
    os.execvpe(state['command'][0], state['command'], state['environment'])


def run(command):
    """Return the normalized child's status, or None if no re-exec is needed.

    Only a zero hard watchdog is normalized. Legitimate finite budgets remain
    unchanged. The ordinary gate still owns build admission and memory limits.
    """
    if sys.platform != 'linux' or resource.getrlimit(resource.RLIMIT_RTTIME)[1] != 0:
        return None
    runtime = pathlib.Path(os.environ.get('XDG_RUNTIME_DIR', ''))
    info = runtime.stat()
    if (not runtime.is_absolute() or not stat.S_ISDIR(info.st_mode)
            or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700):
        raise ValueError('a private XDG_RUNTIME_DIR is required to normalize RLIMIT_RTTIME')
    unit = 'romacs-gate-rlimits-' + uuid.uuid4().hex
    cancelled = []

    def cancel(number, _frame):
        if not cancelled:
            cancelled.append(number)

    def stop():
        subprocess.run(['systemctl', '--user', 'stop', unit], stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, timeout=150, check=False)

    old = {}
    process = None
    try:
        for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            old[number] = signal.signal(number, cancel)
        # The full environment never enters argv, unit properties or persistent
        # storage. A killed owner can leave only a private runtime-dir file.
        with tempfile.TemporaryDirectory(prefix='romacs-gate-rlimits-', dir=runtime) as directory:
            mask = os.umask(0)
            os.umask(mask)
            limits = sorted({getattr(resource, name) for name in dir(resource)
                             if name.startswith('RLIMIT_') and name != 'RLIMIT_RTTIME'})
            state = dict(command=command, environment=dict(os.environ), cwd=os.getcwd(), umask=mask,
                         limits=[(number, resource.getrlimit(number)) for number in limits],
                         affinity=sorted(os.sched_getaffinity(0)), nice=os.getpriority(os.PRIO_PROCESS, 0))
            fd, path = tempfile.mkstemp(dir=directory)
            with os.fdopen(fd, 'w') as stream:
                json.dump(state, stream)
            launcher = ['systemd-run', '--user', '--quiet', '--wait', '--pipe', '--collect',
                        '--unit=' + unit, '--property=LimitRTTIME=infinity',
                        '--property=KillMode=control-group', '--property=TimeoutStopSec=120s',
                        sys.executable, '-I', str(pathlib.Path(__file__).resolve()), '--restore', path]
            if cancelled:
                return 128 + cancelled[0]
            process = subprocess.Popen(launcher, start_new_session=True)
            try:
                while True:
                    # A first stop may precede unit registration. Recheck until
                    # the launcher settles; never signal by a recycled PID.
                    if cancelled:
                        stop()
                    try:
                        result = process.wait(timeout=0.1)
                        break
                    except subprocess.TimeoutExpired:
                        continue
                if cancelled:
                    stop()
                return 128 + cancelled[0] if cancelled else result
            finally:
                if process.poll() is None:
                    try:
                        stop()
                    finally:
                        process.wait(timeout=150)
    finally:
        for number, handler in old.items():
            signal.signal(number, handler)


if __name__ == '__main__':
    try:
        if len(sys.argv) != 3 or sys.argv[1] != '--restore':
            raise ValueError('resource handoff required')
        restore(sys.argv[2])
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'gate: RED (resource normalization failed: {error})', file=sys.stderr, flush=True)
        sys.exit(1)
