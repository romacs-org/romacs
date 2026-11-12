#!/usr/bin/env python3
"""Per-key cost of the live GUI: evaluator instructions and key-to-paint latency.

Each session is a fresh private headless labwc and a fresh `romacs-vm-gui -Q
[--loadup] text.txt`, with private HOME/XDG/TMPDIR and no DISPLAY.  Keys are
typed with wtype into that compositor only.  Under --loadup the session
first sets `auto-save-timeout' nil with M-:, so no idle auto-save of the
typed text lands in the second idle phase.  A session runs, in order:

  idle, self-insert (N x `a'), C-n (N), C-f (N), idle, then C-x C-s.

Counted sessions count the evaluator thread alone (`perf stat -t`,
instructions:u) over each phase; latency sessions run without perf and
with --trace-latency, and read key-enqueue -> render-observed samples: one
per key, each rendered or explicitly unchanged, or the phase is refused.

A phase counts only if
  * exactly N command replies arrived, each naming the key's bound command
    (the binary's `romacs-binding:` lines name C-n's) with no signal;
  * point after the phase is where the commands put it (simulated on the text);
  * a paint completed for a presentation submitted after the last command
    (counting stops only then; a frame nothing changed is not repainted, so
    a phase whose commands signalled only settles); fewer than N paints
    marks it coalesced;
  * the counter is numeric and ran 100% of the time.
A refused phase is reported apart, with its reason, never as a result.
Paints per key are reported too: each paint is a full layout.  The session
counts only if the two idle phases agree within 2x (or differ by
under IDLE_SLACK instructions), the saved file is byte-identical to the text
the commands produced, and the counted thread is still the evaluator.
Their median idle rate is subtracted from each phase.  Before the first idle
the evaluator must be quiet for --settle seconds, past the visit's undo
boundary timer; the timer's later firing (armed by the first self-insert,
10s on) is ordinary typing cost and lands in a key phase.  Before the
second idle it must be quiet again, once blink-cursor-mode has stopped
blinking after the last phase's keys.

With --classic the same M-: also turns `romacs-classic-chrome' on, so
the session draws GNU's look instead of its panes (VM2-PANE-CHROME).
With --line-numbers TYPE it also numbers the buffer's lines
(`display-line-numbers-type' TYPE, e.g. relative), so the key phases pay
for a line-number column redrawn as point moves.
With --state, a `post-command-hook' installed by --eval records after
every command the collections so far and their seconds (`gcs-done',
`gc-elapsed') and the selected window's start, end, point and body and
total heights in pixels, and writes them to the session's state.el at
C-x C-s; --mode sets the output's size (default 1600x1000).
With --setup FORM (repeatable) the same M-: also evaluates FORM.
With --host-calls the same kind of hook records `romacs--host-calls'
after every command, and before it saves the session scrolls (C-v) a few
times, runs M-x auto-fill-mode typed in full, and searches (C-s, six
letters, C-s twice, RET).  The report then gives, per command, the calls
the evaluator made out of the VM that stay with it (`local') and those
that reach the display side, another thread or a file (`cross'), each by
name: the mean over the commands that followed the same command, so one
command's count is its own body, plus the redisplay and the idle time
before it.  A command read in the minibuffer or during a search is named
with that (`minibuffer self-insert-command').  Each of the two prompts
is also reported whole (`host_calls_per_prompt_key'): its calls from the
key that opens it to the command that ends it, over the keys typed.
With --sample FILE the session visits a copy of FILE, under its name (so
its extension picks the major mode, e.g. a tree-sitter mode), instead of
--lines lines of prose; FILE must have no tabs, as C-n's expected point
assumes.
With --populate the visited file is tracked in a Git repository (so
`vc-mode' shows a branch), the buffer is in `flymake-mode' with a backend
that reports three fixed diagnostics once, and `isearch-lazy-count' is
on: the key phases then type in a buffer whose mode line shows them; a
fourth phase, region, sets the mark (C-SPC, not counted) and moves N C-f
with the region active.

usage: perkey_live.py --binary PATH --output DIR [--regime loadup|workspace]
                      [--lines 200] [--sessions 5] [--latency-sessions 1]
                      [--record PHASE] [--tools DIR:DIR] [--classic] [--state]
                      [--mode WxH] [--populate] [--line-numbers TYPE] [--sample FILE]
                      [--host-calls] [--setup FORM]...
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
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
        'save': ['-M', 'ctrl', '-k', 'x', '-k', 's', '-m', 'ctrl'],
        'scroll': ['-M', 'ctrl', 'v', '-m', 'ctrl'], 'M-x': ['-M', 'alt', 'x', '-m', 'alt'],
        'C-s': ['-M', 'ctrl', 's', '-m', 'ctrl'], 'RET': ['-k', 'Return']}
SCROLLS = 4
EXTENDED = 'auto-fill-mode'
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
                return 0
            if not fields[0].isdigit():
                raise Invalid(f'counter not counted: {line}')
            if float(fields[4] or 0) < 100.0:
                raise Invalid(f'counter ran under 100%: {line}')
            return int(fields[0])
    raise Invalid(f'no instructions line: {output!r}')


def delivery(replies, command, keys):
    """Problems with the command replies of one phase, which ran KEYS keys of COMMAND."""
    problems = []
    if len(replies) != keys:
        problems.append(f'{len(replies)} command replies for {keys} keys')
    for reply in replies:
        if reply.get('native_command') != command or reply.get('signal') or reply.get('native_error'):
            problems.append(f'unexpected reply {reply}')
            break
    return problems


def latency_check(samples, keys):
    """Render times of one phase's latency-key SAMPLES, how many keys needed no
    paint, and problems.  Each of the KEYS keys must have exactly one sample,
    the phase's samples one contiguous run of key sequences, and each sample
    either a render time or the binary's explicit `unchanged' (a frame nothing
    changed is not repainted).  A sample with neither, such as a key with no
    enqueue time, is refused, never dropped."""
    problems = []
    sequences = [s.get('key_sequence') for s in samples]
    if len(samples) != keys:
        problems.append(f'{len(samples)} latency samples for {keys} keys')
    if len(set(sequences)) != len(sequences) or None in sequences:
        problems.append(f'latency samples without unique key sequences: {sequences}')
    elif sequences and sorted(sequences) != list(range(min(sequences), min(sequences) + len(sequences))):
        problems.append(f'latency samples are not one run of keys: {sorted(sequences)}')
    rendered = [s['key_to_render_observed_ns'] for s in samples if s.get('key_to_render_observed_ns') is not None]
    unchanged = sum(1 for s in samples if s.get('key_to_render_observed_ns') is None and s.get('unchanged') is True)
    if len(rendered) + unchanged != len(samples):
        problems.append(f'{len(samples) - len(rendered) - unchanged} latency samples with no render time')
    return rendered, unchanged, problems


def idle_rate(idles):
    """Median idle rate of [(instructions, seconds)], refusing idles that disagree."""
    rates = [count / seconds for count, seconds in idles]
    low, high = min(rates), max(rates)
    if high > 2 * low and (high - low) * max(s for _, s in idles) > IDLE_SLACK:
        raise Invalid(f'idle phases disagree: {rates}')
    return statistics.median(rates)


def spread(values):
    middle = statistics.median(values)
    result = {'median': middle, 'min': min(values), 'max': max(values), 'n': len(values),
              'spread': (max(values) - min(values)) / middle if middle else None}
    result['flag'] = result['spread'] is not None and result['spread'] > SPREAD_FLAG
    return result


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(fraction * len(ordered)))]


def records(path):
    """JSON objects among the lines of PATH, in order."""
    found = []
    for line in path.read_text(errors='replace').splitlines():
        if line.startswith('{'):
            try:
                found.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return found


def bindings(stderr_text):
    """`romacs-binding: KEY\\tGNU\\tRUNS` lines as KEY -> RUNS."""
    found = {}
    for line in stderr_text.splitlines():
        if line.startswith('romacs-binding: '):
            key, _, runs = line[len('romacs-binding: '):].split('\t')
            found[key] = runs
    return found


def sexps(text):
    """The Lisp forms printed in TEXT, as nested lists of atoms' names."""
    stack = [[]]
    for token in re.findall(r'[()]|[^\s()]+', text):
        if token == '(':
            stack.append([])
        elif token == ')':
            done = stack.pop()
            stack[-1].append(done)
        else:
            stack[-1].append(token)
    return stack[0]


def search_word(text):
    """Six lowercase letters TEXT has at least three times, not all one letter, to search for."""
    runs = (word for word in dict.fromkeys(re.findall(r'[a-z]{6}', text)) if len(set(word)) > 1 and text.count(word) >= 3)
    found = next(runs, None)
    if not found:
        raise Invalid('no six letters to search for')
    return found


def host_call_rows(forms):
    """--host-calls' rows, [COMMAND, CONTEXT, WINDOW-START, [[NAME, SIDE, COUNT]...]]
    after each command, as (command, context, window start, {name: (side, count)})."""
    return [(command, context, start, {name: (side, int(count)) for name, side, count in calls})
            for command, context, start, calls in forms]


def after_setup(rows):
    """ROWS after the session's last M-: (its setup, typed in a minibuffer of its own)."""
    last = max((i for i, row in enumerate(rows) if row[0] == 'eval-expression'), default=-1)
    return rows[last + 1:]


def calls_between(was, now):
    """The calls NOW counts that WAS did not: {name: (side, calls)}."""
    calls = {name: (side, count - was.get(name, (side, 0))[1]) for name, (side, count) in now.items()}
    return {name: call for name, call in calls.items() if call[1]}


def host_call_segments(rows):
    """(command, {name: (side, calls)}) for each command of ROWS that followed
    the same command in the same context, the calls since that one ended.  A
    command not at top level is named with its context.  A scroll that left
    the window where it was (the buffer's end) is none."""
    found = []
    for (before, was_in, start, was), (command, context, end, now) in zip(rows, rows[1:]):
        if (command, context) != (before, was_in) or (command.startswith('scroll-') and start == end):
            continue
        found.append((command if context == 'top' else f'{context} {command}', calls_between(was, now)))
    return found


def host_call_prompts(rows):
    """{context: {name: (side, calls)}} for each prompt of ROWS: the calls from
    the last top-level command before it to the first after it, which is
    the command that ended it."""
    found, opened = {}, None
    for (_, was_in, _, was), (_, context, _, now) in zip(rows, rows[1:]):
        if was_in == 'top' and context != 'top':
            opened = (context, was)
        elif context == 'top' and opened:
            found.setdefault(opened[0], calls_between(opened[1], now))
            opened = None
    return found


def side_total(calls, side):
    return sum(n for s, n in calls.values() if s == side)


def host_calls_per_command(segments):
    """Per command of SEGMENTS: how many, the mean and the most calls on each
    side, and each name's side and mean."""
    report = {}
    for command in dict.fromkeys(command for command, _ in segments):
        runs = [calls for name, calls in segments if name == command]
        entry = {'commands': len(runs), 'by_name': {}}
        for side in ('local', 'cross'):
            totals = [side_total(calls, side) for calls in runs]
            entry[side] = {'mean': statistics.mean(totals), 'max': max(totals)}
        for name in sorted({name for calls in runs for name in calls}):
            side = next(calls[name][0] for calls in runs if name in calls)
            entry['by_name'][name] = {'side': side, 'mean': sum(calls.get(name, (side, 0))[1] for calls in runs) / len(runs)}
        report[command] = entry
    return report


def host_calls_per_prompt_key(prompts, keys):
    """Per prompt context of PROMPTS, one {context: calls} per session: the
    calls on each side and by name per key typed, KEYS[context] of them."""
    report = {}
    for context in dict.fromkeys(context for prompt in prompts for context in prompt):
        runs = [prompt[context] for prompt in prompts if context in prompt]
        typed = keys[context] * len(runs)
        entry = {'prompts': len(runs), 'keys_each': keys[context], 'by_name': {}}
        for side in ('local', 'cross'):
            entry[side] = sum(side_total(calls, side) for calls in runs) / typed
        for name in sorted({name for calls in runs for name in calls}):
            side = next(calls[name][0] for calls in runs if name in calls)
            entry['by_name'][name] = {'side': side, 'per_key': sum(calls.get(name, (side, 0))[1] for calls in runs) / typed}
        report[context] = entry
    return report


def text_point(stderr_records):
    """Point of the first (text) window in the last cursor publication."""
    published = [r for r in stderr_records if r.get('diagnostic') == 'cursor-publication']
    if not published or not published[-1]['points']:
        raise Invalid('no cursor publication')
    return published[-1]['points'][0]['point_byte']


# The live session.

class Session:
    def __init__(self, args, root, latency):
        self.args, self.root, self.latency = args, root, latency
        self.compositor = self.keyboard = self.process = None

    def env(self):
        home = self.root / 'home'
        path = os.environ.get('PATH', '/usr/bin:/bin')
        env = {'PATH': f'{self.args.tools}:{path}' if self.args.tools else path,
               'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1', 'XDG_RUNTIME_DIR': str(self.root),
               'XDG_CONFIG_HOME': str(home / 'config'), 'XDG_CACHE_HOME': str(home / 'cache'),
               'XDG_DATA_HOME': str(home / 'data'), 'XDG_STATE_HOME': str(home / 'state'),
               'TMPDIR': str(self.root), 'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman',
               'WLR_HEADLESS_OUTPUTS': '1', 'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop'}
        for key in ('ROMACS_LISP_DIR', 'ROMACS_RUNTIME_ROOT', 'ROMACS_WORLD', 'ROMACS_GNU_SOURCE'):
            if key in os.environ:
                env[key] = os.environ[key]
        if 'ROMACS_GATE_FONTCONFIG' in os.environ:
            env['FONTCONFIG_FILE'] = os.environ['ROMACS_GATE_FONTCONFIG']
        return env

    def start(self):
        work, config = self.root / 'work', self.root / 'config'
        for directory in (self.root / 'home', work, config):
            directory.mkdir()
        (config / 'rc.xml').write_text(live.RC_XML)
        sample = getattr(self.args, 'sample', None)
        self.file = work / (sample.name if sample else 'text.txt')
        self.file.write_text(sample_text(self.args))
        if self.args.populate:
            git = ['git', '-C', str(work), '-c', 'user.name=perkey', '-c', 'user.email=perkey@localhost', '-c', 'commit.gpgsign=false']
            for command in (['init', '-q', '-b', 'main'], ['add', self.file.name], ['commit', '-q', '-m', 'text']):
                subprocess.run(git + command, check=True, capture_output=True, env={'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(self.root / 'home')})
        self.stdout, self.stderr = self.root / 'binary.stdout', self.root / 'binary.stderr'
        self.environ = env = self.env()
        log = (self.root / 'compositor.log').open('w')
        self.compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or 'labwc', '-C', str(config)],
                                           env=env, stdout=log, stderr=log)
        self.wait(lambda: any(p.is_socket() for p in self.root.glob('wayland-*')), 20, 'compositor')
        env['WAYLAND_DISPLAY'] = next(p.name for p in self.root.glob('wayland-*') if p.is_socket())
        subprocess.run([shutil.which('wlr-randr', path=env['PATH']) or 'wlr-randr', '--output', 'HEADLESS-1',
                        '--custom-mode', self.args.mode], env=env, check=True, timeout=15)
        # A held key gives the seat a keyboard before the client starts.
        self.keyboard = subprocess.Popen(['wtype', '-s', '600000', '-k', 'Shift_L'], env=env,
                                         stdout=log, stderr=log)
        time.sleep(.3)
        argv = [str(self.args.binary), '-Q'] + (['--loadup'] if self.args.regime == 'loadup' else []) \
            + (['--trace-latency'] if self.latency else []) \
            + (['--eval', self.state_form()] if self.args.state else []) \
            + (['--eval', self.host_calls_form()] if getattr(self.args, 'host_calls', False) else []) + [str(self.file)]
        self.process = subprocess.Popen(argv, env=env, cwd=work, stdin=subprocess.DEVNULL,
                                        stdout=self.stdout.open('w'), stderr=self.stderr.open('w'))
        self.wait(lambda: any(r.get('ready') for r in records(self.stdout)), self.args.timeout, 'ready')
        self.tid = self.evaluator()
        if self.args.regime == 'loadup':
            self.no_idle_auto_save()
        self.quiesce()
        stderr_text = self.stderr.read_text(errors='replace')
        self.commands = bindings(stderr_text)
        self.renderer = next((l[len('romacs: gpu '):] for l in stderr_text.splitlines()
                              if l.startswith('romacs: gpu ')), None)

    def setup_form(self):
        forms = ['(setq auto-save-timeout nil)']
        if self.args.classic:
            forms.append('(romacs-classic-chrome 1)')
        # Harnesses that build their own arguments (theme_live.py,
        # isolates_live.py) number no lines.
        numbers = getattr(self.args, 'line_numbers', None)
        if numbers:
            forms.append(f'(progn (setq display-line-numbers-type (quote {numbers})) (display-line-numbers-mode 1))')
        if self.args.populate:
            forms += ['(require (quote flymake))',
                      '(setq trusted-content :all flymake-no-changes-timeout nil flymake-start-on-save-buffer nil isearch-lazy-count t)',
                      '(defun perkey--diagnostics (report &rest _) (funcall report (list'
                      ' (flymake-make-diagnostic (current-buffer) 1 2 :error "e")'
                      ' (flymake-make-diagnostic (current-buffer) 3 4 :warning "w")'
                      ' (flymake-make-diagnostic (current-buffer) 5 6 :note "n"))))',
                      '(setq-local flymake-diagnostic-functions (list (quote perkey--diagnostics)))',
                      '(flymake-mode 1)']
        if getattr(self.args, 'split', 1) > 1:
            # The measured buffer stays selected; the other windows show
            # other buffers, each drawing its own mode line.
            # Stacked, full width: line motion measures what it does alone.
            forms.append('(let ((others (list (get-buffer-create "*perkey-other*") (messages-buffer))))'
                         ' (save-selected-window (select-window (split-window-below))'
                         ' (switch-to-buffer (car others))'
                         f'{" (select-window (split-window-below)) (switch-to-buffer (cadr others))" if getattr(self.args, 'split', 1) > 2 else ""}))')
        forms += getattr(self.args, 'setup', None) or []
        return f"(progn {' '.join(forms)})"

    def state_form(self):
        """--state's hook: a record per command, written out at C-x C-s."""
        path = self.root / 'state.el'
        return ('(progn (defvar perkey--states nil) (add-hook (quote post-command-hook) (lambda ()'
                ' (push (list this-command gcs-done gc-elapsed (window-start) (window-end) (point)'
                ' (window-body-height nil t) (window-pixel-height)) perkey--states)'
                ' (when (memq this-command (quote (save-buffer romacs-gui-save-buffer)))'
                f' (with-temp-file "{path}"'
                ' (dolist (s (reverse perkey--states)) (prin1 s (current-buffer)) (terpri (current-buffer))))))))')

    def host_calls_form(self):
        """--host-calls' hook: the counts after each command, written out at C-x C-s."""
        path = self.root / 'host-calls.el'
        return ('(progn (defvar perkey--calls nil) (add-hook (quote post-command-hook) (lambda ()'
                ' (push (list this-command (cond ((minibufferp) (quote minibuffer)) ((bound-and-true-p isearch-mode) (quote isearch)) (t (quote top)))'
                ' (window-start) (romacs--host-calls)) perkey--calls)'
                ' (when (memq this-command (quote (save-buffer romacs-gui-save-buffer)))'
                f' (let ((mode major-mode)) (with-temp-file "{path}"'
                ' (prin1 mode (current-buffer)) (terpri (current-buffer))'
                ' (dolist (s (reverse perkey--calls)) (prin1 s (current-buffer)) (terpri (current-buffer)))))))))')

    def no_idle_auto_save(self):
        """M-: (setq auto-save-timeout nil).  read_char's idle auto-save of the
        typed text, about 30 s after the last key, would land in the second
        idle phase, which measures background, not typing.  -Q reads no init,
        so the session sets it through its own command loop."""
        before = len(records(self.stdout))
        self.type(['-M', 'alt', ':', '-m', 'alt'])
        self.type([self.setup_form()])
        self.type(['-k', 'Return'])
        done = lambda: [r for r in records(self.stdout)[before:] if r.get('native_command') == 'eval-expression']
        self.wait(done, self.args.timeout, 'M-: (setq auto-save-timeout nil)')
        if any(r.get('signal') or r.get('native_error') for r in done()):
            raise Invalid(f'M-: (setq auto-save-timeout nil) signalled: {done()}')

    def evaluator(self):
        for task in Path(f'/proc/{self.process.pid}/task').iterdir():
            if (task / 'comm').read_text().strip().startswith('romacs-evaluat'):
                return int(task.name)
        raise Invalid('no evaluator thread')

    def quiesce(self):
        """Wait until the evaluator has used no CPU for --settle seconds: work
        started by startup (a timer's repaint) must not land in an idle phase."""
        stat = Path(f'/proc/{self.process.pid}/task/{self.tid}/stat')
        ticks = lambda: sum(int(f) for f in stat.read_text().rsplit(')', 1)[1].split()[11:13])
        end = time.monotonic() + self.args.timeout
        last, since = ticks(), time.monotonic()
        while time.monotonic() - since < self.args.settle:
            if time.monotonic() > end:
                raise Invalid('the evaluator never went quiet')
            time.sleep(.1)
            if (now := ticks()) != last:
                last, since = now, time.monotonic()

    def wait(self, pred, timeout, what):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if pred():
                return
            if self.process is not None and self.process.poll() is not None:
                raise Invalid(f'{what}: binary exited {self.process.returncode}')
            time.sleep(.05)
        raise Invalid(f'timed out waiting for {what}')

    def type(self, parts):
        subprocess.run(['wtype', *parts], env=self.environ, check=True, timeout=30)

    def run_keys(self, parts, count):
        """Type COUNT keys; return their replies once a later presentation has
        painted.  A command that signalled may change nothing on screen, and
        an unchanged frame is not repainted: its phase is refused anyway, so it
        only settles."""
        before = len(records(self.stdout))
        for _ in range(count):
            self.type(parts)
            time.sleep(self.args.gap)
        self.wait(lambda: len([r for r in records(self.stdout)[before:] if 'native_command' in r
                               or 'native_error' in r]) >= count, self.args.timeout, 'command replies')
        replies = [r for r in records(self.stdout)[before:] if 'native_command' in r or 'native_error' in r]
        if any(r.get('signal') or r.get('native_error') for r in replies):
            time.sleep(1)
            return replies
        last = replies[-1]['presentation'] if replies else 0
        self.wait(lambda: any(r.get('diagnostic') == 'paint' and r['presentation'] > last
                              for r in records(self.stderr)), self.args.timeout, 'paint after the last command')
        return replies

    def prompt(self, keys):
        """Type KEYS, one prompt from the key that opens it to the one that ends
        it, then let its last redisplay and idle work land."""
        for parts in keys:
            self.type(parts)
            time.sleep(self.args.gap)
        time.sleep(1)

    def counted(self, action, idle=False):
        """(instructions, seconds, result) of ACTION on the evaluator thread, or seconds only."""
        if self.latency:
            started = time.monotonic()
            result = action()
            return None, time.monotonic() - started, result
        out = self.root / f'perf-{time.monotonic_ns()}.txt'
        perf = subprocess.Popen(['perf', 'stat', '-x,', '-e', 'instructions:u', '-t', str(self.tid), '-o', str(out)],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(.3)
        started = time.monotonic()
        result = action()
        seconds = time.monotonic() - started
        time.sleep(.3)
        perf.send_signal(signal.SIGINT)
        perf.wait(timeout=10)
        return perf_count(out.read_text(), idle), seconds + .6, result

    def phase(self, name, command, keys):
        start = len(records(self.stderr))
        count, seconds, replies = self.counted(lambda: self.run_keys(KEYS[name], keys))
        stderr_records = records(self.stderr)
        new = stderr_records[start:]
        paints = [r for r in new if r.get('diagnostic') == 'paint']
        problems = delivery(replies, command, keys)
        latencies, unchanged = [], 0
        if self.latency:
            latencies, unchanged, missing = latency_check(
                [r for r in new if r.get('diagnostic') == 'latency-key'], keys)
            problems += missing
        return {'instructions': count, 'seconds': seconds, 'replies': replies,
                'paints': len(paints), 'point': text_point(stderr_records), 'latency_ns': latencies,
                'unchanged_keys': unchanged, 'problems': problems}

    def measure(self):
        keys = self.args.keys
        text, point = 'a' * keys + sample_text(self.args), 0
        idle = lambda: time.sleep(keys * self.args.gap)
        result = {'tid': self.tid, 'commands': self.commands, 'renderer': self.renderer, 'phases': {}}
        if text_point(records(self.stderr)) != 0:
            raise Invalid('point is not at the start of the visited file')
        first = self.counted(idle, idle=True)
        commands = {'self-insert': 'self-insert-command', 'C-f': 'forward-char', 'C-n': self.commands.get('[14]')}
        commands['region'] = 'forward-char'
        for name in phases(self.args):
            if name == 'region':
                mark = self.run_keys(KEYS['mark'], 1)
                result['mark'] = delivery(mark, 'set-mark-command', 1)
            phase = self.phase(name, commands[name], keys)
            want = expected_point(name, text, point, keys)
            if phase['point'] != want:
                phase['problems'].append(f"point {phase['point']}, expected {want}")
            point = phase['point']
            phase['coalesced'] = phase['paints'] < keys
            result['phases'][name] = phase
        # The keys' own aftermath is not background: blink-cursor-mode blinks
        # for blink-cursor-blinks after the last key, then stops.
        self.quiesce()
        second = self.counted(idle, idle=True)
        if not self.latency:
            result['idle'] = [(first[0], first[1]), (second[0], second[1])]
        counting = getattr(self.args, 'host_calls', False)
        if counting:
            self.run_keys(KEYS['scroll'], SCROLLS)
            word = search_word(text)
            self.prompt([KEYS['M-x']] + [['-k', 'minus'] if letter == '-' else [letter] for letter in EXTENDED] + [KEYS['RET']])
            self.prompt([KEYS['C-s']] + [[letter] for letter in word] + [KEYS['C-s'], KEYS['C-s'], KEYS['RET']])
            result['prompt_keys'] = {'minibuffer': len(EXTENDED) + 2, 'isearch': len(word) + 4}
        save = self.run_keys(KEYS['save'], 1)
        result['save'] = save
        problems = delivery(save, self.commands.get('[24 19]'), 1) + result.get('mark', [])
        self.wait(lambda: self.file.read_text() == text, 10, 'saved text')
        result['problems'] = problems
        if counting:
            calls = self.root / 'host-calls.el'
            self.wait(calls.exists, 10, 'host calls')
            result['major_mode'], *forms = sexps(calls.read_text())
            rows = after_setup(host_call_rows(forms))
            result['host_calls'] = host_call_segments(rows)
            result['host_call_prompts'] = host_call_prompts(rows)
            if sorted(result['host_call_prompts']) != ['isearch', 'minibuffer']:
                problems.append(f"prompts counted: {sorted(result['host_call_prompts'])}")
        if self.evaluator() != self.tid:
            raise Invalid('evaluator thread changed')
        if self.args.record and not self.latency:
            result['recorded'] = self.record(self.args.record)
        return result

    def record(self, name):
        """Profile N more keys of phase NAME, after the measured phases."""
        data = self.args.output / f'{name}.perf.data'
        if data.exists():
            return None
        # Flat: call graphs need perf_event_paranoid < 2.
        perf = subprocess.Popen(['perf', 'record', '-e', 'instructions:u', '-t', str(self.tid), '-o', str(data)],
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(.5)
        replies = self.run_keys(KEYS[name], self.args.keys)
        time.sleep(.3)
        perf.send_signal(signal.SIGINT)
        perf.wait(timeout=120)
        return {'data': str(data), 'replies': len(replies)}

    def stop(self):
        for proc in (self.process, self.keyboard, self.compositor):
            live.stop(proc)


def session(args, index, latency):
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/1000'))
    with tempfile.TemporaryDirectory(prefix='perkey-', dir=runtime) as temporary:
        root = Path(temporary)
        root.chmod(0o700)
        live_session = Session(args, root, latency)
        try:
            live_session.start()
            result = live_session.measure()
            result['valid'] = True
        except (Invalid, AssertionError, subprocess.SubprocessError) as error:
            result = {'valid': False, 'error': repr(error)}
        finally:
            live_session.stop()
            kept = args.output / f"{'latency' if latency else 'counted'}-{index}"
            kept.mkdir()
            for name in ('binary.stdout', 'binary.stderr', 'compositor.log', 'state.el', 'host-calls.el'):
                if (root / name).exists():
                    shutil.copy(root / name, kept / name)
    return result


def per_key(sessions, keys):
    """Per phase, idle-subtracted instructions per key: of the phases that
    delivered (`valid'), and of those that did not (`refused', with why);
    and completed paints per key, since each costs a full layout."""
    found = {}
    for result in sessions:
        rate = idle_rate(result['idle'])
        for name in result['phases']:
            phase = result['phases'][name]
            entry = found.setdefault(name, {'valid': [], 'refused': [], 'why': [], 'paints': []})
            entry['refused' if phase['problems'] else 'valid'].append(
                (phase['instructions'] - rate * phase['seconds']) / keys)
            entry['paints'].append(phase['paints'] / keys)
            entry['why'] += [p for p in phase['problems'][:1] if p not in entry['why']]
    return {name: {'valid': spread(e['valid']) if e['valid'] else None, 'paints_per_key': spread(e['paints']),
                   'refused': {'per_key': spread(e['refused']), 'why': e['why']} if e['refused'] else None}
            for name, e in found.items()}


def invalid(result):
    """Why a whole session's numbers cannot be used, or None."""
    if not result['valid']:
        return result['error']
    problems = list(result['problems'])
    if 'idle' in result:
        try:
            idle_rate(result['idle'])
        except Invalid as error:
            problems.append(str(error))
    return problems or None


def latencies(sessions):
    found = {}
    for name in dict.fromkeys(name for result in sessions for name in result['phases']):
        phases = [result['phases'][name] for result in sessions if not result['phases'][name]['problems']]
        values = [v for phase in phases for v in phase['latency_ns']]
        if values:
            found[name] = {'p50_ms': percentile(values, .5) / 1e6, 'p95_ms': percentile(values, .95) / 1e6,
                           'max_ms': max(values) / 1e6, 'samples': len(values),
                           'unchanged_keys': sum(phase['unchanged_keys'] for phase in phases)}
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--regime', choices=('loadup', 'workspace'), default='loadup')
    parser.add_argument('--lines', type=int, default=200)
    parser.add_argument('--sample', type=Path, help='visit a copy of this file (its name picks the mode) instead of --lines of prose')
    parser.add_argument('--keys', type=int, default=40)
    parser.add_argument('--gap', type=float, default=.25, help='seconds between keys')
    parser.add_argument('--sessions', type=int, default=5)
    parser.add_argument('--latency-sessions', type=int, default=1)
    # GNU's undo-auto--boundary-timer fires 10s after the visit's first change.
    parser.add_argument('--settle', type=float, default=12, help='quiet evaluator seconds before measuring')
    parser.add_argument('--timeout', type=float, default=180)
    parser.add_argument('--record', choices=PHASES, help='after the first counted session, profile this phase')
    parser.add_argument('--tools', help='directories with labwc, wlr-randr and wtype, PATH-style')
    parser.add_argument('--classic', action='store_true', help="measure under `romacs-classic-chrome'")
    parser.add_argument('--line-numbers', choices=('t', 'relative'), help='number the buffer\'s lines of this type')
    parser.add_argument('--populate', action='store_true', help='a Git-tracked flymake buffer, and a region phase')
    parser.add_argument('--split', type=int, default=1, choices=(1, 2, 3), help='windows in the frame, each with its own buffer')
    parser.add_argument('--state', action='store_true', help='record collections and the window around each phase')
    parser.add_argument('--mode', default='1600x1000', help="the output's size")
    parser.add_argument('--setup', action='append', help='a form the M-: setup also evaluates (--regime loadup)')
    parser.add_argument('--host-calls', action='store_true', help='count the calls out of the VM per command, by side')
    args = parser.parse_args()
    if (args.line_numbers or args.setup) and args.regime != 'loadup':
        parser.error('--line-numbers and --setup need --regime loadup, whose M-: setup evaluates them')
    args.binary = args.binary.resolve()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True)
    counted = [session(args, i, False) for i in range(args.sessions)]
    timed = [session(args, i, True) for i in range(args.latency_sessions)]
    rejected = {f'counted-{i}': why for i, r in enumerate(counted) if (why := invalid(r))}
    rejected |= {f'latency-{i}': why for i, r in enumerate(timed) if (why := invalid(r))}
    good = [r for r in counted if not invalid(r)]
    report = {'binary': str(args.binary), 'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(),
              'regime': args.regime, 'lines': args.lines, 'sample': str(args.sample) if args.sample else None, 'keys_per_phase': args.keys, 'gap_s': args.gap,
              'counter': 'instructions:u, evaluator thread, idle-subtracted',
              'latency': 'key enqueue to render observed (--trace-latency), perf-free sessions',
              'instructions_per_key': per_key(good, args.keys) if good else None,
              'refused_phases': {f'{kind}-{i}': {n: p['problems'] for n, p in r['phases'].items() if p['problems']}
                                 for kind, rs in (('counted', counted), ('latency', timed))
                                 for i, r in enumerate(rs) if r['valid']},
              'latency_ms': latencies([r for r in timed if not invalid(r)]),
              'host_calls_per_command': host_calls_per_command(
                  [tuple(s) for r in counted + timed if not invalid(r) for s in r.get('host_calls', [])]) or None,
              'major_mode': sorted({r['major_mode'] for r in counted + timed if 'major_mode' in r}),
              'host_calls_per_prompt_key': host_calls_per_prompt_key(
                  [r['host_call_prompts'] for r in counted + timed if not invalid(r) and 'host_call_prompts' in r],
                  {'minibuffer': len(EXTENDED) + 2, 'isearch': 10}) or None,
              'renderer': sorted({r['renderer'] or 'unknown' for r in counted + timed if r['valid']}),
              'coalesced': sorted({name for r in counted + timed if r['valid']
                                   for name, p in r['phases'].items() if p['coalesced']}),
              'valid_sessions': len(good), 'rejected': rejected, 'sessions': counted, 'latency_sessions': timed}
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    print(json.dumps({k: report[k] for k in ('regime', 'lines', 'instructions_per_key', 'latency_ms', 'major_mode', 'host_calls_per_command', 'host_calls_per_prompt_key',
                                             'renderer', 'coalesced', 'valid_sessions', 'rejected')}, indent=1))
    refused = any(report['refused_phases'].values())
    return 0 if len(good) == args.sessions and not rejected and not refused else 1


if __name__ == '__main__':
    raise SystemExit(main())
