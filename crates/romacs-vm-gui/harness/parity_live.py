#!/usr/bin/env python3
"""The machinery the side-by-side parity harnesses share.

A harness (gnosis_parity_live.py, hermes_parity_live.py) names its
workflows and how each engine's private tree and init are made; this module
starts each engine in its own private headless labwc, drives the same keys
into both through wtype, asks parity_probe.el for the same observable state
after every step and compares it.  Each harness's docstring states its
COMPARISON POLICY.
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
PROBE = HARNESS / 'parity_probe.el'
# The gate's headless tools: labwc, wlr-randr, wtype.
TOOLS = os.environ.get('ROMACS_HEADLESS_TOOLS', '/nix/store/klrgwbvpwd7napzdhacw1nhj8xmpbpdc-romacs-gate-headless-tools/bin')
GRID = (120, 33)  # the root window's body columns and lines in both frames
# Romacs's interactive defaults off (VM2-OOB-DEFAULTS), in an init that GNU
# also loads; a -Q session never turns them on.
ROMACS_DEFAULTS_OFF = "(when (fboundp 'romacs-defaults-mode) (romacs-defaults-mode -1))\n"
# Completion as GNU's engine has it after loading rlisp/romacs-completion-defaults.el,
# which lacks the fzf style and romacs-complete, set in both engines' inits:
# Romacs's own defaults complete with fzf (DIVERGENCES.md VM2-FZF-DEFAULT) in
# romacs-complete's layer (VM2-COMPLETE-DEFAULT), and these harnesses compare
# GNU's completion, displayed by fido-vertical.  Romacs's other interactive
# defaults (rlisp/romacs-defaults.el, VM2-OOB-DEFAULTS) go off with their
# switch, so both engines edit with GNU's values.
GNU_COMPLETION = ("(setq completion-styles '(basic partial-completion emacs22)\n"
                  "      completion-category-overrides '((file (styles partial-completion))))\n"
                  "(when (fboundp 'romacs-complete-mode) (romacs-complete-mode -1) (fido-vertical-mode 1))\n"
                  + ROMACS_DEFAULTS_OFF)
RC_XML = '''<?xml version="1.0"?>
<labwc_config><core><gap>0</gap></core>
<windowRules><windowRule identifier="*" serverDecoration="no"><action name="Maximize"/></windowRule></windowRules>
</labwc_config>
'''
# The pinned GNU executable; set by main.
GNU = None


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


# --- private trees and Lisp -------------------------------------------------

def private_tree(engine, base, tools, prefix=None):
    """A fresh private root for ENGINE: HOME, XDG, TMPDIR, the probe's
    directory and the compositor environment, named PREFIX (default
    ENGINE-) and a random suffix.  Returns (ROOT, HOME, ENV)."""
    root = Path(tempfile.mkdtemp(prefix=prefix or f'{engine}-', dir=base))
    root.chmod(0o700)
    home = root / 'home'
    for directory in ('home', 'work', 'probe', 'tmp', 'home/.config', 'home/.cache', 'home/.local/share',
                      'home/.local/state'):
        (root / directory).mkdir(parents=True)
    env = {'PATH': f'{tools}:{os.environ["PATH"]}', 'HOME': str(home), 'LANG': 'C.UTF-8', 'ROMACS_DIAGNOSTICS': '1',
           'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / '.config'),
           'XDG_CACHE_HOME': str(home / '.cache'), 'XDG_DATA_HOME': str(home / '.local/share'),
           'XDG_STATE_HOME': str(home / '.local/state'), 'TMPDIR': str(root / 'tmp'),
           'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
           'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop', 'PARITY_PROBE_DIR': str(root / 'probe'),
           'GSETTINGS_BACKEND': 'memory', 'DBUS_SESSION_BUS_ADDRESS': 'unix:path=/nonexistent', 'NO_AT_BRIDGE': '1',
           'GDK_BACKEND': 'wayland',
           # The headless pixman compositor presents no Vulkan surface: wgpu
           # always ends on GL (llvmpipe) here, and probing the Vulkan loader
           # and its drivers first costs ~250 ms of every launch's first paint.
           'WGPU_BACKEND': 'gl'}
    if os.environ.get('ROMACS_GATE_FONTCONFIG'):
        env['FONTCONFIG_FILE'] = os.environ['ROMACS_GATE_FONTCONFIG']
    return root, home, env


def lisp_copy(base, name, sources, compile_):
    """SOURCES copied into one directory under BASE, cached by their names
    and bytes, and with COMPILE_ byte-compiled there by the pinned GNU."""
    digest = hashlib.sha256(b''.join(p.name.encode() + p.read_bytes() for p in sources)).hexdigest()[:16]
    target = base / f'{name}-{digest}'
    if (target / 'done').exists():
        return target
    staging = Path(tempfile.mkdtemp(prefix=f'{name}-', dir=base))
    for source in sources:
        shutil.copy(source, staging / source.name)
    if compile_:
        subprocess.run([GNU, '-Q', '--batch', '-L', str(staging), '--eval', '(setq byte-compile-warnings nil)',
                        '-f', 'batch-byte-compile', *sorted(str(p) for p in staging.glob('*.el'))],
                       check=True, capture_output=True, timeout=900)
    (staging / 'done').write_text(digest)
    staging.rename(target)
    return target


def gnu_engine(root, home, env, init, lisp, timeout, tools, cls=None):
    """Pinned GNU pgtk on INIT, as a user's ~/.config/emacs/init.el;
    LISP is the package directory its paths are normalised from.  GNU's
    native compiler, which Romacs lacks, compiles nothing: neither the
    packages (the harness byte-compiles them) nor the trampolines advising
    a primitive needs (*Native-compile-Log*).  CLS, if given, is the
    Engine subclass to start."""
    (home / '.config/emacs').mkdir()
    (home / '.config/emacs/early-init.el').write_text(
        ';; -*- lexical-binding: t -*-\n(setq package-enable-at-startup nil native-comp-jit-compilation nil'
        ' native-comp-enable-subr-trampolines nil)\n')
    (home / '.config/emacs/init.el').write_text(init)
    (home / '.config/emacs/custom.el').write_text('')
    return (cls or Engine)('gnu', [GNU, '--font', 'Iosevka-14'], root, lisp, env, timeout).start(tools)


def prepare_world(romacs, env, cwd):
    """The world `ROMACS --loadup' boots from in ENV, made usable with no
    window (`--world-build'), as an installer prepares it before a user's
    first launch: the seconds that took, or None when it could not be
    (the launch then boots from source, and says so)."""
    started = time.time()
    done = subprocess.run([romacs, '--loadup', '--world-build'], env=env, cwd=cwd,
                          stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=900)
    return round(time.time() - started, 3) if done.returncode == 0 else None


def romacs_engine(romacs, root, home, env, init, lisp, timeout, tools, cls=None, args=()):
    """`romacs-vm-gui --loadup ARGS' on INIT, as ~/.config/romacs/init.el
    unless INIT is None, from a world prepared first (`prepare_world', its
    seconds in `world_seconds'), as a user's HOME holds one; CLS as
    `gnu_engine'."""
    (home / '.config/romacs').mkdir()
    if init is not None:
        (home / '.config/romacs/init.el').write_text(init)
    env.update(ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14')
    for key in ('ROMACS_GNU_SOURCE', 'ROMACS_RUNTIME_ROOT', 'ROMACS_LISP_DIR'):
        if os.environ.get(key):
            env[key] = os.environ[key]
    world_seconds = prepare_world(romacs, env, root / 'work')
    engine = (cls or Engine)('romacs', [romacs, '--loadup', *args], root, lisp, env, timeout).start(tools)
    engine.world_seconds = world_seconds
    return engine


# --- normalisation and comparison --------------------------------------------

def normalise(value, engine, window, rewrite=None):
    """The path and wall-clock rules every harness's policy shares: the
    engine's private root and Lisp directory become <root> and <lisp>, and
    numbers that read as epoch seconds, milliseconds or microseconds within
    WINDOW become <time>.  REWRITE, if given, maps each string after that."""
    low, high = window
    if isinstance(value, list):
        return [normalise(v, engine, window, rewrite) for v in value]
    if isinstance(value, Sym):
        return value
    if isinstance(value, str):
        value = value.replace(str(engine.root), '<root>')
        if engine.lisp is not None:
            value = value.replace(str(engine.lisp), '<lisp>')
        return rewrite(value) if rewrite else value
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


# --- keys and readiness -------------------------------------------------------

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


# --- running workflows ---------------------------------------------------------

def run_step(engines, step, perf_enabled, seed):
    """Drive STEP into each engine in turn; return per-engine timing."""
    result = {}
    if step.get('text_from'):
        step = dict(step, trigger=[T(str(engines[0].ask(step['text_from'])).strip())])
    for engine in engines:
        # Both engines run GNU's generator (lisp random), but other Lisp, the
        # probe's included, draws from it at different rates; reseed per step
        # so ids drawn during it match.
        engine.ask(f'(progn (random {lisp_string(seed + step["name"])}) (parity-probe-step))')
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


class Harness:
    """What one harness supplies: its WORKFLOWS (name -> steps), LAUNCH
    (engine, base, args -> started Engine), the SNAPSHOT form the probe
    evaluates, the random SEED prefix of each step, an optional string
    REWRITE its policy adds to `normalise', and an optional CHECK
    (engines -> {'errors': [...], ...}) run after a workflow's last step,
    whose errors fail it.  WRITES names the workflows that change the
    backend's data, which run only when named."""

    def __init__(self, workflows, launch, snapshot='(parity-probe-snapshot)', seed='parity-', rewrite=None, check=None, writes=()):
        self.workflows, self.launch, self.snapshot, self.seed, self.rewrite = workflows, launch, snapshot, seed, rewrite
        self.check, self.writes = check, set(writes)


def run_workflow(harness, name, args, base, report):
    entry = {'workflow': name, 'steps': [], 'status': 'FAIL'}
    report['workflows'].append(entry)
    engines = []
    try:
        for engine in ('gnu', 'romacs'):
            engines.append(harness.launch(engine, base, args))
        entry['startup_seconds'] = {e.name: round(e.startup_seconds, 3) for e in engines}
        for engine in engines:
            engine.fit_grid()
            engine.ask('(progn (parity-probe-mark-messages) t)')
        window = (time.time() - 2 * 86400, time.time() + 2 * 86400)
        snap = lambda engine: normalise(engine.ask(harness.snapshot, 120), engine, window, harness.rewrite)
        entry['idle_cpu_percent'] = idle_cpu(engines)
        initial, geometry = compare(*[snap(e) for e in engines])
        entry['steps'].append({'step': 'initial', 'differences': initial, 'geometry': geometry})
        for step in harness.workflows[name]:
            record = {'step': step['name']}
            entry['steps'].append(record)
            record['timing'] = run_step(engines, step, args.perf, harness.seed)
            snapshots = [snap(e) for e in engines]
            record['differences'], record['geometry'] = compare(*snapshots)
            if args.keep_snapshots:
                record['snapshots'] = snapshots
            if step['name'] == args.until:
                break
        entry['status'] = 'PASS' if not any(s['differences'] for s in entry['steps']) else 'DIFFER'
        if harness.check:
            entry['check'] = harness.check(engines)
            if entry['check'].get('errors'):
                entry['status'] = 'FAIL'
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


def main(harness, description, add_arguments=lambda parser: None):
    """Parse the shared options and ADD_ARGUMENTS's, then run each selected
    workflow of HARNESS, writing --output/report.json after each."""
    global GNU
    parser = argparse.ArgumentParser(description=description, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE'))
    add_arguments(parser)
    parser.add_argument('--output', required=True)
    parser.add_argument('--work', default=os.environ.get('TMPDIR', '/tmp'))
    parser.add_argument('--tools', default=TOOLS)
    parser.add_argument('--workflow', action='append')
    parser.add_argument('--timeout', type=float, default=300)
    parser.add_argument('--perf', action='store_true', help='count instructions with perf stat')
    parser.add_argument('--keep', action='store_true', help='keep the private trees')
    parser.add_argument('--keep-snapshots', action='store_true')
    parser.add_argument('--until', metavar='STEP', help='end each workflow after its step STEP')
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
        for name in args.workflow or [n for n in harness.workflows if n not in harness.writes]:
            entry = run_workflow(harness, name, args, base, report)
            print(name, entry['status'], entry.get('error', ''), flush=True)
            (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    finally:
        (output / 'report.json').write_text(json.dumps(report, indent=1, ensure_ascii=False) + '\n')
    return 0 if all(w['status'] == 'PASS' for w in report['workflows']) else 1
