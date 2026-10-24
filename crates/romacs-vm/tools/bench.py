#!/usr/bin/env python3
"""Exact existing slope bodies; GNU reads forms, Rust alone compiles/executes.

Every accepted PMU sample includes a known-answer check and scheduling metadata.
No dependency on perf(1), the inherited evaluator, or its byte compiler.
"""
import argparse
from typing import Any
import ctypes
import fcntl
import importlib.util
import json
import hashlib
import os
from pathlib import Path
import platform
import signal
import statistics
import struct
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
GNU = "/usr/bin/emacs"
SOURCE = Path.home() / ".hermes/cache/scratch/romacs/vmbench/slope_bench.py"
COUNTER = Path.home() / "Projects/rust/romacs/scripts/romacs_accept_counter.py"


def load(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def read_form(source):
    # Reader reuse, not a new Lisp reader: JSON is just already-read data transport.
    form = """(progn (set-language-environment "UTF-8") (set-terminal-coding-system 'utf-8-unix) (require 'json)
      (defun vm2-read (s)
        (let* ((p (read-from-string s)) (tail (substring s (cdr p))))
          (when (string-match-p "[^ \t\n\r]" tail) (error "Trailing read input"))
          (car p)))
      (defun vm2-json (x)
        (cond ((integerp x) x) ((symbolp x) (symbol-name x))
              ((stringp x) (list :string x))
              ((vectorp x) (list :vector (vconcat (mapcar #'vm2-json x))))
              ((consp x) (vconcat (mapcar #'vm2-json x)))
              (t (error "Unsupported read datum"))))
      (princ (decode-coding-string (json-serialize (vm2-json (vm2-read %s))) 'utf-8)))""" % json.dumps(source, ensure_ascii=False)
    return subprocess.check_output([GNU, "-Q", "--batch", "--eval", form], text=True).encode()


def gnu_command(body):
    # Same byte-compile-and-call wrapper as slope_bench.py, with result printed.
    return [GNU, "-Q", "--batch", "--eval", "(setq lexical-binding t)",
            "--eval", f"(princ (funcall (byte-compile (lambda () {body}))))"]


class Unscheduled(RuntimeError):
    pass


def validate(count, enabled, running):
    # Integer comparison, no floating-point threshold rounding; impossible times fail.
    if count <= 0 or enabled <= 0 or running != enabled:
        raise Unscheduled(f"count={count} enabled={enabled} running={running}")


def guarded(command, data, attributes, pressure=0):
    event = None
    competitors = []
    reaped = False
    with tempfile.TemporaryFile() as inp, tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        inp.write(data)
        inp.seek(0)
        child = os.fork()
        if child == 0:
            try:
                for fd, stream in enumerate((inp, out, err)):
                    os.dup2(stream.fileno(), fd)
                os.kill(os.getpid(), signal.SIGSTOP)
                os.execvp(command[0], command)
            except BaseException:
                os._exit(127)
        try:
            _, stopped = os.waitpid(child, os.WUNTRACED)
            if not os.WIFSTOPPED(stopped):
                reaped = True
                raise RuntimeError("measurement child did not stop")
            libc = ctypes.CDLL(None, use_errno=True)
            attr = (ctypes.c_ubyte * 128).from_buffer(attributes())
            syscall = {"x86_64": 298, "aarch64": 241}[platform.machine()]
            # Test hook: occupy task PMU slots before installing the measured event.
            for _ in range(pressure):
                fd = libc.syscall(syscall, ctypes.byref(attr), child, -1, -1, 8)
                if fd < 0:
                    error = ctypes.get_errno()
                    raise OSError(error, os.strerror(error))
                competitors.append(fd)
                fcntl.ioctl(fd, 0x2400, 0)
            event = libc.syscall(syscall, ctypes.byref(attr), child, -1, -1, 8)
            if event < 0:
                error = ctypes.get_errno()
                raise OSError(error, os.strerror(error))
            fcntl.ioctl(event, 0x2403, 0)  # reset
            fcntl.ioctl(event, 0x2400, 0)  # attr.disabled=1: enable is mandatory
            started = time.perf_counter_ns()
            os.kill(child, signal.SIGCONT)
            _, status, usage = os.wait4(child, 0)
            elapsed = time.perf_counter_ns() - started
            reaped = True
            fcntl.ioctl(event, 0x2401, 0)  # disable
            raw = os.read(event, 24)
            if len(raw) != 24:
                raise Unscheduled("short PMU record")
            count, enabled, running = struct.unpack("QQQ", raw)
            validate(count, enabled, running)
            out.seek(0)
            err.seek(0)
            stdout, stderr = out.read().decode(), err.read().decode()
            rc = os.waitstatus_to_exitcode(status)
            if rc:
                raise RuntimeError(f"child exit {rc}: {stderr}")
            return dict(count=count, enabled=enabled, running=running,
                        stdout=stdout.strip(), stderr=stderr, exit=rc,
                        wall_ns=elapsed, user_cpu_s=usage.ru_utime, system_cpu_s=usage.ru_stime,
                        minor_faults=usage.ru_minflt, major_faults=usage.ru_majflt,
                        peak_rss_kib=usage.ru_maxrss)
        finally:
            if event is not None and event >= 0:
                os.close(event)
            for fd in competitors:
                os.close(fd)
            if not reaped:
                os.kill(child, signal.SIGKILL)
                os.waitpid(child, 0)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--binary", default=str(ROOT / "target/release/romacs-vm"))
    p.add_argument("--source", type=Path, default=SOURCE)
    p.add_argument("--counter", type=Path, default=COUNTER)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--reps", type=int, default=3)
    p.add_argument("--gc-floor", type=int, help="Explicit pacing floor for a collector-cost control run")
    args = p.parse_args()
    if args.reps < 1:
        p.error("reps must be positive")
    cases = load(args.source, "original_slopes").CASES
    attributes = load(args.counter, "original_counter").attributes
    vm_command = [args.binary] + ([] if args.gc_floor is None else ['--gc-floor', str(args.gc_floor)])
    report: dict[str, Any] = dict(gc_floor=args.gc_floor, method="median separate N/2N process counts; slope / N", reps=args.reps,
                  resolution_instructions=36, source=str(args.source), counter=str(args.counter),
                  gnu=subprocess.check_output([GNU, "--version"], text=True).splitlines()[0],
                  samples=[], rejections=[], cases=[], load_before=Path('/proc/loadavg').read_text(),
                  binary_sha256=hashlib.sha256(Path(args.binary).read_bytes()).hexdigest())
    args.output.parent.mkdir(parents=True, exist_ok=True)

    def save():
        args.output.write_text(json.dumps(report, indent=2) + "\n")

    def measure(command, data, expected, label):
        samples = []
        for rep in range(args.reps):
            sample = None
            for attempt in range(5):
                try:
                    sample = guarded(command, data, attributes)
                    break
                except Unscheduled as e:
                    report["rejections"].append(dict(label=label, rep=rep, attempt=attempt, error=str(e)))
                    save()
                    if attempt == 4:
                        raise
                    time.sleep(attempt + 1)
            assert sample is not None
            if sample["stdout"] != expected:
                raise RuntimeError(f"known-answer failure {label}: {sample['stdout']!r} != {expected!r}")
            sample.update(label=label, rep=rep, expected=expected)
            report["samples"].append(sample)
            save()
            samples.append(sample["count"])
        return statistics.median(samples)

    try:
        for name, n, body in cases:
            if name == "buf-insert":
                continue
            row = dict(name=name, n=n, body=body, values={}, medians={})
            for factor in (1, 2):
                source = body.replace("N", str(n * factor))
                data = read_form(source)
                expected = subprocess.check_output(gnu_command(source), text=True).strip()
                got = subprocess.check_output(vm_command, input=data).decode().strip()
                if got != expected:
                    raise RuntimeError(f"known-answer failure {name}/{factor}: {got!r} != {expected!r}")
                row["values"][str(factor)] = expected
                for engine, command, payload in (("vm2", vm_command, data),
                                                  ("gnu", gnu_command(source), b"")):
                    row["medians"][f"{engine}/{factor}"] = measure(command, payload, expected, f"{name}/{engine}/{factor}")
            for engine in ("vm2", "gnu"):
                slope = row["medians"][f"{engine}/2"] - row["medians"][f"{engine}/1"]
                if slope <= report['resolution_instructions']:
                    raise RuntimeError(f"slope below resolution {name}/{engine}: {slope}")
                row[f"{engine}_instructions_per_iteration"] = slope / n
            row["ratio"] = row["vm2_instructions_per_iteration"] / row["gnu_instructions_per_iteration"]
            report["cases"].append(row)
            save()
            print(f"{name:12} vm2={row['vm2_instructions_per_iteration']:10.3f} "
                  f"gnu={row['gnu_instructions_per_iteration']:10.3f} ratio={row['ratio']:.3f}", flush=True)
        if len(report["cases"]) != len([c for c in cases if c[0] != "buf-insert"]):
            raise RuntimeError("case count mismatch")
        report["complete"] = True
    except BaseException as e:
        report["complete"] = False
        report["error"] = str(e)
        raise
    finally:
        report['load_after'] = Path('/proc/loadavg').read_text()
        save()


if __name__ == "__main__":
    main()
