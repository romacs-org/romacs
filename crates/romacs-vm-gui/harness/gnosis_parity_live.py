#!/usr/bin/env python3
"""Gnosis in pinned GNU and in Romacs GUI --loadup, side by side, on a DB copy.

Each workflow starts a fresh pair of sessions: pinned GNU Emacs (pgtk) and
`romacs-vm-gui --loadup`, each in its own private headless labwc with a private
HOME/XDG/TMPDIR and its own copy of the owner's Gnosis database.  pgtk under
labwc, not -nw over a pty: both engines then take the same Wayland key events
from wtype through the same compositor and both are graphical frames, so
`display-graphic-p' code paths, faces and images are the ones the owner uses.

Both inits load gnosis_parity_probe.el, whose timer answers Lisp requests
through files.  The harness types each step's keys into both sessions, waits
for the step's own readiness predicate, then reads the same observable state
from both and compares it under COMPARISON POLICY below.  Every difference is
reported; the policy only names what is platform geometry or wall-clock.

Timing: each step's trigger keys are timed from the moment wtype starts to the
first probe answer in which the step is ready (input drained and predicate
true).  The probe polls every 25ms, so times carry up to 25ms of quantisation;
`perf stat -e instructions:u' attached to the engine over the same interval
counts its user-space instructions, including the probe's idle polling.

usage: gnosis_parity_live.py --romacs BIN --gnu EMACS --db DB --db-repo REPO
                             --output DIR [--workflow NAME]... [--perf]
DB is a copy of the owner's gnosis.db, never the live file; its images/ and
assets/ siblings are copied with it.  REPO is a Git repository whose only
commit holds that copy as gnosis.db: each session clones it (--shared), so
gnosis's automatic commits after a review have a history to land on, as in
the owner's gnosis-dir.  Nothing here writes DB, REPO or their directory.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

HARNESS = Path(__file__).resolve().parent
ROOT = HARNESS.parents[2]
PROBE = HARNESS / 'gnosis_parity_probe.el'
# The gate's headless tools: labwc, wlr-randr, wtype.
TOOLS = os.environ.get('ROMACS_HEADLESS_TOOLS', '/nix/store/klrgwbvpwd7napzdhacw1nhj8xmpbpdc-romacs-gate-headless-tools/bin')
GRID = (120, 33)  # the root window's body columns and lines in both frames
RC_XML = '''<?xml version="1.0"?>
<labwc_config><core><gap>0</gap></core>
<windowRules><windowRule identifier="*" serverDecoration="no"><action name="Maximize"/></windowRule></windowRules>
</labwc_config>
'''

# ---------------------------------------------------------------------------
# COMPARISON POLICY
#
# Compared exactly: buffers and their order, the selected buffer, point, text
# (visible text verbatim, whole-buffer SHA-1), narrowing, read-only and
# modified flags, major mode, local and global minor modes, the mode line,
# header lines, text properties and overlays on visible text, the window tree,
# each window's buffer, start, point, hscroll and body size in columns/lines,
# the minibuffer, the echo area, the *Messages* tail since the workflow began,
# every non-custom `gnosis' variable, and the step's SQL query results.
#
# Normalised, with the reason:
#   * Paths: each engine's private root and its gnosis Lisp directory become
#     <root> and <lisp>.  The trees are copies at different places.
#   * Wall-clock: numbers that read as epoch seconds, milliseconds or
#     microseconds within the run (+/- 2 days) become <time>.  The engines
#     run the same step seconds apart.
#   * Geometry, recorded and reported apart: the frame's own columns and
#     lines (its chrome, fringes and a partial last line, differ by engine;
#     the root window's body is held to GRID instead), window pixel edges,
#     and window-end.  Line pixel heights follow each engine's font metrics,
#     so a window with taller lines (images, :height faces) may show fewer of
#     them; text and properties are compared over the shorter visible span.
#   * Typing speed: gnosis's drill reports words per minute from the keys'
#     arrival times; "WPM: N" becomes "WPM: <rate>".
#   * Randomness: each step starts from (random "gnosis-parity-STEP") on both
#     engines, whose generators agree (lisp random is GNU's in Romacs too).
#   * Identities: gnosis's review event and session ids hash the clock and
#     the process id (gnosis-scheduler-event-id), so the probe names each by
#     the order it was made, <event-id-N>.
#   * Settling: a step is observed once input is drained, its predicate
#     holds and no one-shot timer is due within 3s (parity-probe-quiet), so
#     work a step deferred to a timer has run on both sides.
# Nothing else is ignored; an unexplained difference is a Romacs bug.
# ---------------------------------------------------------------------------

DB_QUERIES = [
    'SELECT count(*) FROM themata',
    'SELECT count(*) FROM review_event',
    'SELECT count(*) FROM review_session',
    'SELECT count(*) FROM study_session',
    'SELECT count(*) FROM thema_tag',
    'SELECT count(*) FROM nodes',
    # The workflow's own thema; gnosis draws its id from `random', seeded alike.
    "SELECT id, type, keimenon, hypothesis, answer FROM themata WHERE keimenon LIKE '%Parity keimenon%'",
    # Event and session ids are random per engine; their other columns are not.
    'SELECT thema_id, session_sequence, reviewed_at_us, review_day, detected_outcome, accepted_outcome,'
    ' override_count, first_review, thema_type, content_sha256, scheduler_version'
    ' FROM review_event ORDER BY reviewed_at_us DESC LIMIT 3',
    'SELECT source_kind, source_ref, started_at_us, started_review_day, planned_count, event_count, status'
    ' FROM review_session ORDER BY started_at_us DESC LIMIT 1',
    'SELECT s.* FROM scheduler_state s JOIN review_event e USING (thema_id)'
    ' ORDER BY e.reviewed_at_us DESC LIMIT 3',
    'SELECT * FROM review_day ORDER BY review_day DESC LIMIT 1',
]


# --- s-expression reader for the probe's prin1 output -----------------------

class Sym(str):
    """A Lisp symbol, distinct from a string."""
    def __repr__(self):
        return f'Sym({str.__repr__(self)})'


TOKEN = re.compile(r'''\s*(?:(?P<open>[(\[])|(?P<close>[)\]])|(?P<quote>')|"(?P<string>(?:[^"\\]|\\.)*)"|(?P<atom>(?:[^\s()\[\]"';\\]|\\.)+))''', re.S)
ESCAPES = {'n': '\n', 't': '\t', 'r': '\r', 'e': '\x1b', 'a': '\a', 'f': '\f', 'b': '\b', 'v': '\v',
           'd': '\x7f', 's': ' ', '\n': ''}


def unescape(body):
    out, i = [], 0
    while i < len(body):
        c = body[i]
        if c != '\\':
            out.append(c)
            i += 1
            continue
        n = body[i + 1]
        if n in ESCAPES:
            out.append(ESCAPES[n])
            i += 2
        elif n in '01234567':
            m = re.match(r'[0-7]{1,3}', body[i + 1:])
            out.append(chr(int(m.group(), 8)))
            i += 1 + len(m.group())
        elif n == 'x':
            m = re.match(r'[0-9a-fA-F]+', body[i + 2:])
            out.append(chr(int(m.group(), 16)))
            i += 2 + len(m.group())
            if i < len(body) and body[i] == '\\' and i + 1 < len(body) and body[i + 1] == ' ':
                i += 2
        elif n == '^':
            out.append(chr(ord(body[i + 2].upper()) ^ 0x40))
            i += 3
        else:
            out.append(n)
            i += 2
    return ''.join(out)


def atom(text):
    if re.fullmatch(r'[+-]?\d+\.?', text):
        return int(text.rstrip('.'))
    if re.fullmatch(r'[+-]?(\d+\.\d*|\.\d+|\d+)(e[+-]?\d+)?', text) or 'e+INF' in text or 'e+NaN' in text:
        try:
            return float(text.replace('e+INF', 'inf').replace('e+NaN', 'nan'))
        except ValueError:
            pass
    if text == 'nil':
        return None
    return Sym(re.sub(r'\\(.)', r'\1', text))


def read_sexp(text):
    """Read one prin1-printed object; lists become Python lists."""
    stack, position = [[]], 0
    while True:
        match = TOKEN.match(text, position)
        if not match or match.end() == position:
            break
        position = match.end()
        if match['open']:
            stack.append([])
        elif match['close']:
            done = stack.pop()
            stack[-1].append(done)
        elif match['quote']:
            stack[-1].append(Sym('quote'))
        elif match['string'] is not None:
            stack[-1].append(unescape(match['string']))
        else:
            stack[-1].append(atom(match['atom']))
    if text[position:].strip() or len(stack) != 1 or len(stack[0]) != 1:
        raise ValueError(f'unreadable probe output at {position}: {text[position:position + 80]!r}')
    return stack[0][0]


def plist(items):
    return {str(items[i])[1:]: items[i + 1] for i in range(0, len(items) - 1, 2)} if items else {}


def lisp_string(text):
    return json.dumps(text, ensure_ascii=False)


# --- an engine session -------------------------------------------------------

def wait(predicate, timeout, what, process=None, interval=.05):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        value = predicate()
        if value:
            return value
        if process is not None and process.poll() is not None:
            raise RuntimeError(f'{what}: process exited {process.returncode}')
        time.sleep(interval)
    raise TimeoutError(f'timed out after {timeout}s waiting for {what}')


def stop(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


class Engine:
    """One editor in its own private compositor, home and database copy."""

    def __init__(self, name, argv, root, lisp, env, timeout):
        self.name, self.argv, self.root, self.lisp, self.timeout = name, argv, root, lisp, timeout
        self.env, self.serial, self.processes = env, 0, []
        self.probe_dir = root / 'probe'

    def start(self, tools):
        self.tools = tools
        (self.root / 'labwc').mkdir()
        (self.root / 'labwc/rc.xml').write_text(RC_XML)
        log = (self.root / 'compositor.log').open('w')
        self.processes.append(subprocess.Popen([f'{tools}/labwc', '-C', str(self.root / 'labwc')],
                                               env=self.env, stdout=log, stderr=log))
        socket = wait(lambda: next((p for p in self.root.glob('wayland-*') if p.is_socket()), None),
                      15, f'{self.name} compositor', self.processes[0])
        self.env['WAYLAND_DISPLAY'] = socket.name
        self.randr(1280, 900)
        self.processes.append(subprocess.Popen([f'{tools}/wtype', '-s', '86400000', '-k', 'Shift_L'],
                                               env=self.env, stdout=log, stderr=log))
        time.sleep(.3)
        started = time.time()
        output = (self.root / 'engine.log').open('w')
        self.process = subprocess.Popen(self.argv, env=self.env, cwd=self.root / 'work',
                                        stdin=subprocess.DEVNULL, stdout=output, stderr=output)
        self.processes.append(self.process)
        wait(lambda: (self.probe_dir / 'alive').exists(), self.timeout * 4, f'{self.name} init', self.process, .02)
        self.startup_seconds = float((self.probe_dir / 'alive').read_text()) - started
        return self

    def randr(self, width, height):
        subprocess.run([f'{self.tools}/wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', f'{width}x{height}'],
                       env=self.env, check=True, timeout=15, capture_output=True)

    def ask(self, form, timeout=None):
        """Evaluate FORM (Lisp source) in the engine's probe; return its value."""
        self.serial += 1
        request, response = self.probe_dir / 'request.eld', self.probe_dir / 'response.eld'
        if response.exists():
            response.unlink()
        temporary = self.probe_dir / 'request.tmp'
        temporary.write_text(f'({self.serial} . {form})', encoding='utf-8')
        temporary.rename(request)
        wait(response.exists, timeout or self.timeout, f'{self.name} probe answer to {form[:60]}', self.process, .005)
        value = read_sexp(response.read_text(encoding='utf-8'))
        response.unlink()
        if value[0] != self.serial:
            raise RuntimeError(f'{self.name}: answer {value[0]} for request {self.serial}')
        if value[1] != 'ok':
            raise LispError(self.name, form, value[2])
        return value[2]

    def keys(self, *groups):
        for group in groups:
            subprocess.run([f'{self.tools}/wtype', *group], env=self.env, check=True, timeout=30)

    def fit_grid(self):
        """Size the output so the root window's body is GRID columns and lines."""
        for _ in range(4):
            cols, lines, cw, ch = self.ask(
                '(list (window-body-width (frame-root-window)) (window-body-height (frame-root-window))'
                ' (frame-char-width) (frame-char-height))')
            if (cols, lines) == GRID:
                # Then trim any part column or line, which code that measures
                # in pixels (gnosis's centring) would see.
                pw, ph = self.ask('(list (window-body-width (frame-root-window) t) (window-body-height (frame-root-window) t))')
                extra = (pw - cols * cw, ph - lines * ch)
                if extra != (0, 0):
                    width, height = self.output_size()
                    self.randr(width - extra[0], height - extra[1])
                    time.sleep(1)
                return
            width, height = self.output_size()
            self.randr(width + (GRID[0] - cols) * cw, height + (GRID[1] - lines) * ch)
            time.sleep(1)
        raise RuntimeError(f'{self.name}: root window body stays {cols}x{lines}, not {GRID}')

    def output_size(self):
        mode = subprocess.run([f'{self.tools}/wlr-randr'], env=self.env, capture_output=True, text=True).stdout
        current = re.search(r'(\d+)x(\d+) px.*current', mode)
        return (int(current[1]), int(current[2])) if current else (1280, 900)

    def perf(self):
        return subprocess.Popen(['perf', 'stat', '-x', ',', '-e', 'instructions:u', '-p', str(self.process.pid)],
                                stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)

    def stop(self):
        for process in reversed(self.processes):
            stop(process)


class LispError(Exception):
    def __init__(self, engine, form, condition):
        super().__init__(f'{engine}: {form[:80]} signalled {condition!r}')
        self.condition = condition


def idle_cpu(engines, seconds=5):
    """Percent of one core each engine used over SECONDS of idleness: the
    probe's polling and any idle timers, the floor every step's time and
    instruction count stands on."""
    ticks = lambda pid: sum(int(f) for f in Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[11:13])
    before = {e.name: ticks(e.process.pid) for e in engines}
    time.sleep(seconds)
    hz = os.sysconf('SC_CLK_TCK')
    return {e.name: round(100 * (ticks(e.process.pid) - before[e.name]) / hz / seconds, 1) for e in engines}


def perf_result(process):
    if process is None:
        return None
    process.send_signal(signal.SIGINT)
    _, err = process.communicate(timeout=10)
    for line in err.splitlines():
        fields = line.split(',')
        if len(fields) > 2 and 'instructions' in fields[2] and fields[0].isdigit():
            return int(fields[0])
    return None


# --- the shared configuration ------------------------------------------------

def init_source(engine, root, lisp):
    """The init both engines load: identical except where each finds gnosis."""
    # GNU gets the packages Romacs bundles, installed as package.el would:
    # their directories on `load-path' and the same generated autoloads
    # Romacs's --loadup loads.  gnosis and keymap-popup come byte-compiled.
    bundled = [lisp] + [ROOT / 'rlisp' / d for d in ('hermes', 'jabber', 'deps/fsm', 'deps/markdown-mode', 'deps/websocket')]
    load = '' if lisp is None else (
        f'(setq load-path (append (list {" ".join(lisp_string(str(d)) for d in bundled[:1])}) load-path'
        f' (list {" ".join(lisp_string(str(d)) for d in bundled[1:])})))\n'
        f'(load {lisp_string(str(ROOT / "rlisp/romacs-bundled-autoloads.el"))} nil t)\n')
    # GNU takes the settings Romacs's --loadup makes before the user's init:
    # its completion defaults (the same file), its scroll-conservatively
    # (romacs-startup.el) and no GUI chrome Romacs lacks.
    gnu_setup = ('(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
                 '(setq scroll-conservatively 101)\n'
                 f'(load {lisp_string(str(ROOT / "rlisp/romacs-completion-defaults.el"))} nil t)\n') if engine == 'gnu' else ''
    return f''';; -*- lexical-binding: t -*-
{load}{gnu_setup}(setq inhibit-startup-screen t initial-scratch-message nil)
(setq gnosis-dir {lisp_string(str(root / 'gnosis') + '/')})
(windmove-mode 1)
(random "gnosis-parity")
(load {lisp_string(str(PROBE))} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''


def private_tree(engine, base, args):
    root = Path(tempfile.mkdtemp(prefix=f'{engine}-', dir=base))
    root.chmod(0o700)
    home = root / 'home'
    for directory in ('home', 'work', 'probe', 'tmp', 'home/.config', 'home/.cache', 'home/.local/share',
                      'home/.local/state'):
        (root / directory).mkdir(parents=True)
    gitenv = {'HOME': str(home), 'PATH': os.environ['PATH'], 'GIT_CONFIG_NOSYSTEM': '1'}
    (home / '.gitconfig').write_text('[user]\n\tname = parity\n\temail = parity@localhost\n'
                                     '[commit]\n\tgpgsign = false\n[init]\n\tdefaultBranch = master\n')
    subprocess.run(['git', 'clone', '-q', '--shared', str(args.db_repo), str(root / 'gnosis')],
                   env=gitenv, check=True)
    for extra in ('images', 'assets'):
        source = Path(args.db).parent / extra
        if source.is_dir():
            subprocess.run(['cp', '-r', '--reflink=auto', str(source), str(root / 'gnosis' / extra)], check=True)
    env = {'PATH': f'{args.tools}:{os.environ["PATH"]}', 'HOME': str(home), 'LANG': 'C.UTF-8',
           'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / '.config'),
           'XDG_CACHE_HOME': str(home / '.cache'), 'XDG_DATA_HOME': str(home / '.local/share'),
           'XDG_STATE_HOME': str(home / '.local/state'), 'TMPDIR': str(root / 'tmp'),
           'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
           'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop', 'PARITY_PROBE_DIR': str(root / 'probe'),
           'GSETTINGS_BACKEND': 'memory', 'DBUS_SESSION_BUS_ADDRESS': 'unix:path=/nonexistent', 'NO_AT_BRIDGE': '1',
           'GDK_BACKEND': 'wayland'}
    if os.environ.get('ROMACS_GATE_FONTCONFIG'):
        env['FONTCONFIG_FILE'] = os.environ['ROMACS_GATE_FONTCONFIG']
    return root, home, env


def gnu_lisp(base):
    """gnosis and keymap-popup byte-compiled by the pinned GNU, as the owner
    runs them; cached by source hash."""
    sources = sorted((ROOT / 'rlisp/gnosis').glob('*.el')) + [ROOT / 'rlisp/emacs-lisp/keymap-popup.el']
    digest = hashlib.sha256(b''.join(p.name.encode() + p.read_bytes() for p in sources)).hexdigest()[:16]
    target = base / f'gnu-gnosis-{digest}'
    if (target / 'done').exists():
        return target
    staging = Path(tempfile.mkdtemp(prefix='gnu-gnosis-', dir=base))
    for source in sources:
        shutil.copy(source, staging / source.name)
    subprocess.run([GNU, '-Q', '--batch', '-L', str(staging), '--eval', '(setq byte-compile-warnings nil)',
                    '-f', 'batch-byte-compile', *sorted(str(p) for p in staging.glob('*.el'))],
                   check=True, capture_output=True, timeout=900)
    (staging / 'done').write_text(digest)
    staging.rename(target)
    return target


def launch(engine, base, args):
    root, home, env = private_tree(engine, base, args)
    if engine == 'gnu':
        lisp = gnu_lisp(base)
        (home / '.config/emacs').mkdir()
        (home / '.config/emacs/early-init.el').write_text(
            ';; -*- lexical-binding: t -*-\n(setq package-enable-at-startup nil native-comp-jit-compilation nil)\n')
        (home / '.config/emacs/init.el').write_text(init_source(engine, root, lisp))
        (home / '.config/emacs/custom.el').write_text('')
        argv = [GNU, '--font', 'Iosevka-14']
    else:
        lisp = ROOT / 'rlisp/gnosis'
        (home / '.config/romacs').mkdir()
        (home / '.config/romacs/init.el').write_text(init_source(engine, root, None))
        env.update(ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14')
        for key in ('ROMACS_GNU_SOURCE',):
            if os.environ.get(key):
                env[key] = os.environ[key]
        argv = [args.romacs, '--loadup']
    return Engine(engine, argv, root, lisp, env, args.timeout).start(args.tools)


# --- normalisation and comparison --------------------------------------------

def normalise(value, engine, window):
    """Apply the path and wall-clock rules of COMPARISON POLICY."""
    low, high = window
    if isinstance(value, list):
        return [normalise(v, engine, window) for v in value]
    if isinstance(value, Sym):
        return value
    if isinstance(value, str):
        value = value.replace(str(engine.root), '<root>').replace(str(engine.lisp), '<lisp>')
        return re.sub(r'WPM: \d+', 'WPM: <rate>', value)
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if any(low * scale <= value <= high * scale for scale in (1, 1000, 1000000)):
            return '<time>'
    return value


def differences(gnu, romacs, path=''):
    """Paths where two normalised values differ, with both sides."""
    if isinstance(gnu, list) and isinstance(romacs, list) and len(gnu) == len(romacs):
        out = []
        for i, (a, b) in enumerate(zip(gnu, romacs)):
            out += differences(a, b, f'{path}[{i}]')
        return out
    return [] if gnu == romacs else [{'path': path, 'gnu': gnu, 'romacs': romacs}]


def compare(gnu, romacs):
    """Differences between two snapshots under COMPARISON POLICY."""
    g, r = plist(gnu), plist(romacs)
    found, geometry = [], []
    for key in sorted(set(g) | set(r)):
        if key in ('windows', 'variables'):
            continue
        if key == 'frame':
            if g.get(key) != r.get(key):
                geometry.append({'path': key, 'gnu': g.get(key), 'romacs': r.get(key)})
            continue
        found += differences(g.get(key), r.get(key), key)
    gw, rw = g.get('windows') or [], r.get('windows') or []
    if len(gw) != len(rw):
        found.append({'path': 'windows', 'gnu': len(gw), 'romacs': len(rw)})
    for i, (a, b) in enumerate(zip(gw, rw)):
        a, b = plist(a), plist(b)
        for key in ('buffer', 'start', 'point', 'body', 'hscroll', 'dedicated', 'header'):
            found += differences(a.get(key), b.get(key), f'windows[{i}].{key}')
        for key in ('end', 'edges'):
            if a.get(key) != b.get(key):
                geometry.append({'path': f'windows[{i}].{key}', 'gnu': a.get(key), 'romacs': b.get(key)})
        span = min(len(a['text']), len(b['text']))
        if isinstance(a.get('end'), int) and isinstance(b.get('end'), int):
            span = min(span, max(0, min(a['end'], b['end']) - a['start']))
        found += differences(a['text'][:span], b['text'][:span], f'windows[{i}].text')
        clip = lambda runs: [[max(0, s), min(e, span), p] for s, e, p in (runs or []) if s < span]
        found += differences(clip(a.get('runs')), clip(b.get('runs')), f'windows[{i}].runs')
        found += differences(clip(a.get('overlays')), clip(b.get('overlays')), f'windows[{i}].overlays')
    gv = {str(k): v for k, v in g.get('variables') or []}
    rv = {str(k): v for k, v in r.get('variables') or []}
    for name in sorted(set(gv) | set(rv)):
        if name not in rv or name not in gv:
            found.append({'path': f'variables.{name}', 'gnu': gv.get(name, '<unbound>'),
                          'romacs': rv.get(name, '<unbound>')})
        else:
            found += differences(gv[name], rv[name], f'variables.{name}')
    return found, geometry


# --- workflows ----------------------------------------------------------------

def M(key):
    return ['-M', 'alt', key, '-m', 'alt']


def C(key):
    return ['-M', 'ctrl', key, '-m', 'ctrl']


def K(*names):
    group = []
    for name in names:
        group += ['-k', name]
    return group


def T(text):
    return [text]


def mx(command):
    """M-x COMMAND, typed, and the RET that runs it as the timed trigger."""
    return {'keys': [M('x'), T(command)], 'keys_ready': f'(equal (minibuffer-contents-no-properties) {lisp_string(command)})',
            'trigger': [K('Return')]}


BUFFER_IS = '(equal (buffer-name (window-buffer (selected-window))) {})'.format
POPUP_HAS = ('(let ((b (get-buffer "*keymap-popup*"))) (and b (get-buffer-window b)'
             ' (with-current-buffer b (save-excursion (goto-char (point-min)) (search-forward {} nil t)))))').format
DASHBOARD = lisp_string('*Gnosis Dashboard*')
DASHBOARD_RENDERED = (f'(and {BUFFER_IS(DASHBOARD)} (with-current-buffer {DASHBOARD}'
                      ' (and (null gnosis-dashboard--timer) (null gnosis-dashboard--pending-entries))))')
DASHBOARD_READY = (f'(and {DASHBOARD_RENDERED} (with-current-buffer {DASHBOARD} (save-excursion (goto-char (point-min))'
                   ' (and (search-forward "Review streak" nil t) (not (search-forward "Loading statistics" nil t))))))')
MODE_IS = '(eq (buffer-local-value (quote major-mode) (window-buffer (selected-window))) (quote {}))'.format
PROMPT_IS = '(equal (minibuffer-prompt) {})'.format
CONTENTS_ARE = '(equal (minibuffer-contents-no-properties) {})'.format


def key(name, keys, ready='t', **extra):
    """A step whose timed trigger is KEYS (wtype argument groups)."""
    return dict(name=name, trigger=keys, ready=ready, **extra)


def typed(name, text, then, ready, **extra):
    """Type TEXT untimed, then time THEN."""
    return dict(name=name, keys=[T(text)], keys_ready=f'(string-suffix-p {lisp_string(text)} (buffer-substring-no-properties (line-beginning-position) (point)))',
                trigger=then, ready=ready, **extra)


WORKFLOWS = {
    # The dashboard, its menus and its themata list, browsed with Emacs keys.
    'dashboard': [
        dict(name='open', **mx('gnosis-dashboard'), ready=DASHBOARD_READY),
        key('themata-menu', [T('t')], POPUP_HAS('"View all themata"')),
        key('all-themata', [T('a')], f'(and {MODE_IS("gnosis-dashboard-themata-mode")} {DASHBOARD_RENDERED})'),
        key('end-of-buffer', [M('>')]),
        key('beginning-of-buffer', [M('<')]),
        key('back', [T('q')], DASHBOARD_READY),
        key('nodes-menu', [T('n')], POPUP_HAS('"View all nodes"')),
        key('all-nodes', [T('a')], f'(and {MODE_IS("gnosis-dashboard-nodes-mode")} {DASHBOARD_RENDERED})'),
        key('nodes-back', [T('q')], DASHBOARD_READY),
        key('history', [T('H')], f'(and {MODE_IS("gnosis-dashboard-history-mode")} {DASHBOARD_RENDERED})'),
        key('history-back', [T('q')], DASHBOARD_READY),
        # Line motion last: GNU's next-line needs vertical-motion and
        # current-bidi-paragraph-direction, which t3/daily-use-gui-fixes adds.
        key('themata-menu-2', [T('t')], POPUP_HAS('"View all themata"')),
        key('all-themata-2', [T('a')], f'(and {MODE_IS("gnosis-dashboard-themata-mode")} {DASHBOARD_RENDERED})'),
        key('next-line', [C('n')]),
        key('next-line-2', [C('n')]),
        key('previous-line', [C('p')]),
        key('scroll-up', [C('v')]),
    ],
    # Searching and browsing themata and tags from the dashboard.
    'search-tags': [
        dict(name='open', **mx('gnosis-dashboard'), ready=DASHBOARD_READY),
        key('search-prompt', [T(' ')], '(minibufferp (window-buffer (selected-window)))'),
        typed('search', 'vertebral', [K('Return')], f'(and {MODE_IS("gnosis-dashboard-themata-mode")} {DASHBOARD_RENDERED})'),
        key('search-back', [T('q')], DASHBOARD_READY),
        key('themata-menu', [T('t')], POPUP_HAS('"View all tags"')),
        key('all-tags', [T('T')], f'(and {MODE_IS("gnosis-dashboard-tags-mode")} {DASHBOARD_RENDERED})'),
        key('tag-themata', [K('Return')], f'(and {MODE_IS("gnosis-dashboard-themata-mode")} {DASHBOARD_RENDERED})'),
        key('tag-themata-back', [T('q')], f'{MODE_IS("gnosis-dashboard-tags-mode")}'),
    ],
    # A scheduled review of the copy's first due themata: a right answer
    # (this copy's first due cloze is the vertebral arch's laminae), the
    # next question, a wrong answer and its typing drill, then accept & quit.
    'review': [
        dict(name='review-menu', **mx('gnosis-review'), ready=POPUP_HAS('"Due themata"')),
        key('due', [T('d')], '(minibufferp (window-buffer (selected-window)))'),
        typed('answer-right', 'laminae', [K('Return')], POPUP_HAS('"Accept & quit"')),
        key('next', [T('n')], '(minibufferp (window-buffer (selected-window)))'),
        typed('answer-wrong', 'parity', [K('Return')], BUFFER_IS('"*gnosis-monkeytype*"')),
        # The drill shows the answer to type; GNU's buffer says which keys.
        dict(name='drill', text_from='(with-current-buffer "*gnosis-monkeytype*" (buffer-substring-no-properties (point-min) (point-max)))',
             trigger=[], ready=POPUP_HAS('"Accept & quit"')),
        key('accept-quit', [T('q')], BUFFER_IS('"*Gnosis Study Summary*"')),
    ],
    # Adding a basic thema through its Org draft, then finding and editing it.
    'add-edit': [
        dict(name='add', **mx('gnosis-add-thema'), ready=f'(and (minibufferp (window-buffer (selected-window))) {PROMPT_IS(lisp_string("Select type: "))})'),
        # settle: M-x's binding suggestion sits for 2s in a timer, and in
        # Romacs that read swallows the next key until t3/daily-use-gui-fixes
        # (unread-command-events before host input) lands.
        typed('type', 'basic', [K('Return')],
              f'(and {BUFFER_IS(lisp_string("*Gnosis NEW*"))} (save-excursion (forward-line -1) (looking-at-p "\\*\\* Keimenon")))',
              settle=3),
        # Org's heading motion to the Answer heading, then to the line below.
        typed('keimenon', 'Parity keimenon λ', [C('c'), C('n'), C('c'), C('n'), C('e'), C('f')],
              '(save-excursion (forward-line -1) (looking-at-p "\\*\\* Answer"))'),
        typed('answer', 'parity answer', [C('c'), C('c')], '(not (get-buffer "*Gnosis NEW*"))'),
        dict(name='dashboard', **mx('gnosis-dashboard'), ready=DASHBOARD_READY),
        key('search-prompt', [T(' ')], '(minibufferp (window-buffer (selected-window)))'),
        typed('search', 'Parity keimenon', [K('Return')], f'(and {MODE_IS("gnosis-dashboard-themata-mode")} {DASHBOARD_RENDERED})'),
        key('edit', [T('e')], BUFFER_IS('"*Gnosis Edit*"')),
        key('to-keimenon', [C('b')], '(looking-back "λ" 2)'),
        typed('change', ' edited', [C('c'), C('c')], '(not (get-buffer "*Gnosis Edit*"))'),
    ],
}


def run_step(engines, step, window, perf_enabled):
    """Drive STEP into each engine in turn; return per-engine timing."""
    result = {}
    if step.get('text_from'):
        step = dict(step, trigger=[T(str(engines[0].ask(step['text_from'])).strip())])
    for engine in engines:
        # Both engines run GNU's generator (lisp random), but other Lisp, the
        # probe's included, draws from it at different rates; reseed per step
        # so ids gnosis draws during it (thema ids) match.
        engine.ask(f'(progn (random {lisp_string("gnosis-parity-" + step["name"])}) t)')
        if step.get('keys'):
            engine.keys(*step['keys'])
            wait(lambda: plist(engine.ask(f'(parity-probe-ready (lambda () {step.get("keys_ready", "t")}))')).get('ready'),
                 engine.timeout, f'{engine.name} {step["name"]} keys', engine.process, .02)
        counter = engine.perf() if perf_enabled else None
        if counter:
            time.sleep(.25)
        sent = time.time()
        engine.keys(*step['trigger'])
        typed = time.time()
        while True:
            ready = plist(engine.ask(f'(parity-probe-ready (lambda () {step["ready"]}))'))
            if ready.get('ready'):
                break
            if time.time() - sent > engine.timeout:
                raise TimeoutError(f'{engine.name} {step["name"]}: not ready: {ready}')
        result[engine.name] = {'seconds': round(ready['time'] - sent, 4), 'wtype_seconds': round(typed - sent, 4),
                               'instructions': perf_result(counter)}
    time.sleep(step.get('settle', .4))
    return result


def run_workflow(name, args, base, report):
    entry = {'workflow': name, 'steps': [], 'status': 'FAIL'}
    report['workflows'].append(entry)
    engines = []
    try:
        for engine in ('gnu', 'romacs'):
            engines.append(launch(engine, base, args))
        entry['startup_seconds'] = {e.name: round(e.startup_seconds, 3) for e in engines}
        for engine in engines:
            engine.fit_grid()
            engine.ask('(progn (parity-probe-mark-messages) t)')
        window = (time.time() - 2 * 86400, time.time() + 2 * 86400)
        snap = lambda engine: normalise(engine.ask(f'(parity-probe-snapshot \'{lisp_list(DB_QUERIES)})', 120),
                                        engine, window)
        entry['idle_cpu_percent'] = idle_cpu(engines)
        initial, geometry = compare(*[snap(e) for e in engines])
        entry['steps'].append({'step': 'initial', 'differences': initial, 'geometry': geometry})
        for step in WORKFLOWS[name]:
            record = {'step': step['name']}
            entry['steps'].append(record)
            record['timing'] = run_step(engines, step, window, args.perf)
            snapshots = [snap(e) for e in engines]
            record['differences'], record['geometry'] = compare(*snapshots)
            if args.keep_snapshots:
                record['snapshots'] = snapshots
        entry['status'] = 'PASS' if not any(s['differences'] for s in entry['steps']) else 'DIFFER'
    except Exception as error:  # noqa: BLE001
        entry['error'] = repr(error)
        for engine in engines:
            try:
                entry.setdefault('last_state', {})[engine.name] = engine.ask('(parity-probe-snapshot)', 20)
            except Exception as inner:  # noqa: BLE001
                entry.setdefault('last_state', {})[engine.name] = repr(inner)
    finally:
        for engine in engines:
            if args.keep:
                entry.setdefault('roots', {})[engine.name] = str(engine.root)
            for log in ('engine.log', 'compositor.log'):
                if (engine.root / log).exists():
                    shutil.copy(engine.root / log, Path(args.output) / f'{name}-{engine.name}-{log}')
            engine.stop()
            if not args.keep:
                shutil.rmtree(engine.root, ignore_errors=True)
    return entry


def lisp_list(strings):
    return '(' + ' '.join(lisp_string(s) for s in strings) + ')'


def main():
    global GNU
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE'))
    parser.add_argument('--db', required=True, help='a copy of gnosis.db; never the live one')
    parser.add_argument('--db-repo', required=True, help='a Git repository holding that copy as gnosis.db')
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=TOOLS)
    parser.add_argument('--workflow', action='append')
    parser.add_argument('--timeout', type=float, default=300)
    parser.add_argument('--perf', action='store_true', help='count instructions with perf stat')
    parser.add_argument('--keep', action='store_true', help='keep the private trees')
    parser.add_argument('--keep-snapshots', action='store_true')
    args = parser.parse_args()
    # A terminated harness still stops its compositors and editors.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    GNU = args.gnu
    output = Path(args.output)
    output.mkdir(parents=True)
    base = Path(args.work)
    report = {'romacs': args.romacs, 'romacs_sha256': hashlib.sha256(Path(args.romacs).read_bytes()).hexdigest(),
              'gnu': GNU, 'grid': GRID, 'workflows': []}
    try:
        for name in args.workflow or list(WORKFLOWS):
            entry = run_workflow(name, args, base, report)
            print(name, entry['status'], entry.get('error', ''), flush=True)
            (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    return 0 if all(w['status'] == 'PASS' for w in report['workflows']) else 1


GNU = None
if __name__ == '__main__':
    sys.exit(main())
