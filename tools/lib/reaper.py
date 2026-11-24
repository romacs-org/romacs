#!/usr/bin/env python3
"""Run a command as the parent of everything it starts, and end it all.

`reaper.py COMMAND...` makes itself the child subreaper (prctl(2)), so a
process the command starts comes back to it when its own parent ends, even
one in a session of its own: an editor's subprocesses each are (`setsid', as
GNU's are), and `setsid --fork' leaves its child an orphan.  While the command
runs they are reaped as they end, as init would; once it has ended, or SIGTERM
asked this to end it, every one left is killed and reaped before this exits:
none can write where the caller goes on to clean.  It exits as the command
did, by the same signal when one ended it.

This ends as its caller does (SIGTERM), and the command as this does
(SIGKILL).  REAPER_PARENT in the environment names the caller: gone before
that tie was made, nothing runs.  What the command started is beyond reach
once this is killed outright: the kernel gives it to the next subreaper up.
"""
import ctypes
import os
from pathlib import Path
import resource
import signal
import sys

PR_SET_PDEATHSIG = 1
PR_SET_CHILD_SUBREAPER = 36
PARENT = 'REAPER_PARENT'
# What Python ignores for itself, which a command it starts would inherit.
IGNORED = (signal.SIGPIPE, signal.SIGXFSZ)


def prctl(option, value):
    if ctypes.CDLL(None, use_errno=True).prctl(option, value, 0, 0, 0) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


def children():
    """This process's children, ended ones not yet reaped too: its own
    record of them, never another process's."""
    found = []
    for task in Path('/proc/self/task').iterdir():
        try:
            found += [int(pid) for pid in (task / 'children').read_bytes().split()]
        except OSError:
            continue
    return found


def end_all():
    """Kill and reap every process left: a killed one's own children come
    here as it ends, and are found on the next pass.  Once a pass finds
    none, nothing that was started is alive."""
    while left := children():
        for pid in left:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        for pid in left:
            try:
                os.waitpid(pid, 0)
            except ChildProcessError:
                pass


def exit_as(status):
    """Exit with STATUS, a `Popen.returncode': a signal's is raised again."""
    if status >= 0:
        sys.exit(status)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    if -status != signal.SIGKILL:
        signal.signal(-status, signal.SIG_DFL)
    os.kill(os.getpid(), -status)
    sys.exit(128 - status)


def spawn(command):
    """Start COMMAND, to die with this process, its signals at their defaults."""
    me = os.getpid()
    pid = os.fork()
    if pid:
        return pid
    try:
        prctl(PR_SET_PDEATHSIG, signal.SIGKILL)
        if os.getppid() == me:
            for number in (*IGNORED, signal.SIGTERM):
                signal.signal(number, signal.SIG_DFL)
            os.execvp(command[0], command)
    except OSError as error:
        print(f'reaper: {command[0]}: {error}', file=sys.stderr, flush=True)
    finally:
        os._exit(127)


def wait_for(host):
    """HOST's status, reaping as init does meanwhile: an orphan that ends
    while the host runs is gone at once, never left a zombie."""
    while True:
        pid, status = os.wait()
        if pid == host:
            return os.waitstatus_to_exitcode(status)


def main(command):
    # Ignored, children would be reaped unseen and `os.wait' find none.
    signal.signal(signal.SIGCHLD, signal.SIG_DFL)
    prctl(PR_SET_CHILD_SUBREAPER, 1)
    prctl(PR_SET_PDEATHSIG, signal.SIGTERM)
    parent = os.environ.pop(PARENT, None)
    if parent is not None and os.getppid() != int(parent):
        sys.exit('reaper: its caller ended before it started')
    pidfd, asked = None, False

    def stop(_signal, _frame):
        nonlocal asked
        asked = True
        # By pidfd: once the host is reaped, its pid may be another's.
        if pidfd is not None:
            try:
                signal.pidfd_send_signal(pidfd, signal.SIGKILL)
            except ProcessLookupError:
                pass
    signal.signal(signal.SIGTERM, stop)
    pid = spawn(command)
    pidfd = os.pidfd_open(pid)
    if asked:
        stop(None, None)
    status = wait_for(pid)
    end_all()
    exit_as(status)


if __name__ == '__main__':
    main(sys.argv[1:])
