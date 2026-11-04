import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import statistics
import subprocess
import tempfile
import time

import live_oracle as live

PHASES = ('self-insert', 'C-n', 'C-f')
KEYS = {'self-insert': ['a'], 'C-n': ['-M', 'ctrl', 'n', '-m', 'ctrl'],
        'C-f': ['-M', 'ctrl', 'f', '-m', 'ctrl'], 'region': ['-M', 'ctrl', 'f', '-m', 'ctrl'],
        'mark': ['-M', 'ctrl', '-k', 'space', '-m', 'ctrl'],
        'save': ['-M', 'ctrl', '-k', 'x', '-k', 's', '-m', 'ctrl']}
IDLE_SLACK = 1_000_000
SPREAD_FLAG = 0.10


class Invalid(Exception):
    """A measurement that must not be reported as a number."""


# Pure parts: text, expectations, validation and summaries.

def text_of(lines):
    return ''.join(f'line {i} of some ordinary prose text, about sixty bytes long\n' for i in range(lines))


def sample_text(args):
    """The visited file's text: --sample's file, else --lines of prose."""
    sample = getattr(args, 'sample', None)
    return sample.read_text() if sample else text_of(args.lines)


def column(text, point):
    return point - (text.rfind('\n', 0, point) + 1)


def next_line(text, point, goal):
    """Point after GNU's next-line from POINT with goal column GOAL (no wrap, no tabs)."""
    end = text.find('\n', point)
    if end < 0:
        raise Invalid('C-n at the last line')
    start = end + 1
    stop = text.find('\n', start)
    return start + min(goal, (stop if stop >= 0 else len(text)) - start)


def phases(args):
    """The key phases a session runs: --populate adds the region phase."""
    return PHASES + (('region',) if args.populate else ())


def expected_point(name, text, start, keys):
    """0-based point after KEYS keys of phase NAME from START in TEXT."""
    if name == 'C-n':
        point, goal = start, column(text, start)
        for _ in range(keys):
            point = next_line(text, point, goal)
        return point
    if start + keys > len(text):
        raise Invalid(f'{name} past the end of the text')
    return start + keys


def perf_count(output, idle=False):
    """Instructions from `perf stat -x,` OUTPUT; refuse anything but a full-time count.
    An IDLE thread that never ran is `<not counted>` with zero run time: 0."""
    for line in output.splitlines():
        fields = line.split(',')
        if len(fields) > 4 and fields[2].startswith('instructions'):
            if idle and fields[0] == '<not counted>' and fields[3] == '0':
