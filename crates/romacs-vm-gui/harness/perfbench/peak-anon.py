#!/usr/bin/env python3
"""peak-anon.py COMMAND...: run COMMAND, then print its peak private anonymous
memory, which no kernel counter keeps, read from outside it every few
milliseconds so that the run allocates nothing for the measure. A sample: the
true peak is at least this."""
import subprocess
import sys
import time


def anon_kb(pid):
    try:
        with open(f"/proc/{pid}/status") as status:
            for line in status:
                if line.startswith("RssAnon:"):
                    return int(line.split()[1])
    except (OSError, ValueError):
        pass
    return 0


process = subprocess.Popen(sys.argv[1:])
peak = 0
while process.poll() is None:
    peak = max(peak, anon_kb(process.pid))
    time.sleep(0.005)
print(f"peak-anon {peak // 1024} MB")
sys.exit(process.returncode)
