#!/usr/bin/env python3
"""Linux timed instruction wrapper: inherit threads, not helper processes.

perf stat --no-inherit drops GC threads. This wrapper sets perf_event_attr's
inherit_thread bit (Linux >=5.13) instead. FIFO control remains perf-compatible.
The measured child starts stopped; the event is installed before exec, and its
thread-group descendants inherit it. A fork/exec control helper does not.
"""
import ctypes
import fcntl
import os
from pathlib import Path
import platform
import select
import signal
import struct
import sys


def attributes():
    attr = bytearray(128)
    # PERF_TYPE_HARDWARE, size, PERF_COUNT_HW_INSTRUCTIONS; enabled/running times.
    struct.pack_into('IIQQQQQ', attr, 0, 0, len(attr), 1, 0, 0, 3,
                     (1 << 0) | (1 << 1) | (1 << 5) | (1 << 6) | (1 << 35))
    return attr


def run(output, control, ack, command):
    syscall = {'x86_64': 298, 'aarch64': 241}.get(platform.machine())
    if syscall is None:
        raise ValueError('timed counters support Linux x86_64/aarch64 only')
    child = os.fork()
    if child == 0:
        try:
            os.kill(os.getpid(), signal.SIGSTOP)
            os.execvp(command[0], command)
        except BaseException:
            os._exit(127)
    event = ctl = reply = None
    reaped = False
    try:
        os.waitpid(child, os.WUNTRACED)
        libc = ctypes.CDLL(None, use_errno=True)
        attr = (ctypes.c_ubyte * 128).from_buffer(attributes())
        event = libc.syscall(syscall, ctypes.byref(attr), child, -1, -1, 8)
        if event < 0:
            error = ctypes.get_errno()
            raise OSError(error, os.strerror(error))
        ctl = os.open(control, os.O_RDWR | os.O_NONBLOCK)
        reply = os.open(ack, os.O_RDWR | os.O_NONBLOCK)
        os.kill(child, signal.SIGCONT)
        pending = b''
        while True:
            if select.select([ctl], [], [], .01)[0]:
                pending += os.read(ctl, 4096)
                while b'\n' in pending:
                    line, pending = pending.split(b'\n', 1)
                    if line not in (b'enable', b'disable'):
                        raise ValueError('invalid perf control command')
                    fcntl.ioctl(event, 0x2400 if line == b'enable' else 0x2401, 0)
                    os.write(reply, b'ack\n')
            pid, status = os.waitpid(child, os.WNOHANG)
            if pid:
                reaped = True
                break
        count, enabled, running = struct.unpack('QQQ', os.read(event, 24))
        if not count or not enabled or running < enabled * .999:
            raise ValueError('missing/multiplexed timed instruction counter')
        Path(output).write_text(f'{count};;instructions:u;{running};{100 * running / enabled:.3f}\n')
        return os.waitstatus_to_exitcode(status)
    finally:
        for fd in (event, ctl, reply):
            if fd is not None and fd >= 0:
                os.close(fd)
        if not reaped:
            os.kill(child, signal.SIGKILL)
            os.waitpid(child, 0)


if __name__ == '__main__':
    try:
        output, control, ack, *command = sys.argv[1:]
        sys.exit(run(output, control, ack, command))
    except (OSError, ValueError) as error:
        print(f'romacs-accept-counter: {error}', file=sys.stderr)
        sys.exit(2)
