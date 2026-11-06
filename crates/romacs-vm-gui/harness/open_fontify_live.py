#!/usr/bin/env python3
"""Opening a large file and seeing its faces: Romacs against pinned GNU.

Each session is a fresh private headless labwc (on the real GPU when
ROMACS_HARNESS_WLR_ENV names its renderer environment) with one editor,
Iosevka 14, maximized, in a private HOME whose work directory holds a copy
of the file.  Keys are typed with wtype, as a person types them:

  C-x C-f FILE RET          open (a)
  C-v x5                    scroll right after opening (c)
  C-x k RET, C-x C-f FILE RET   kill and re-open (d)
  M->, ten characters       jump to the end and type (c)
  <f9>, then C-n            fontify the whole buffer (b), a key sent into it

The same Lisp runs in both editors (MEASURE below).  `pre-command-hook'
stamps each command's start; `post-command-hook' marks the redisplays so
far and starts a 2 ms timer that polls until a redisplay has finished since
the mark and every character between `window-start' and the updated
`window-end' is `fontified', i.e. has its final faces, which only a
redisplay gives it (jit-lock fontifies what redisplay shows).  Timers never
run inside a redisplay, so a poll that sees one begun sees it finished.
GNU counts redisplays with `pre-redisplay-function'; Romacs, which does not
call it, with `romacs--redisplay-since-mark'.  So "first correct paint" is
measured, in both editors, as the open command's start to the first moment
after a redisplay that the window is fully fontified, within 2 ms, and a
key's latency as its command's start to the first such moment after its
own redisplay.  A command whose window never got there fails the run.  For
Romacs, whose frames are painted by a render thread, --trace-latency also
gives the end of the render that showed the file (`ret_render'), reported
beside it.
<f9> runs `font-lock-ensure' over the buffer and logs its seconds; a C-n
sent 50 ms into it reports how long it waited, from its sending to its
redisplay (nothing, when <f9> was over before it was sent).  RSS (VmRSS,
VmHWM) is read from /proc after opening and after <f9>.

Zed has no hook, so it is measured by pixels: `settle' takes the earliest
frame of the picture the screen then kept for --quiet seconds.  That is a
visual heuristic: faces landing later than that are missed, so a file whose
parse outlasts --quiet needs a longer one.  `romacs-pixel', which has the
hook beside it, fails a run whose settled picture came before the faces
landed.

Romacs sessions run an installation's compiled Lisp: the store beside the
binary is filled first, as an installer does (--no-compile-lisp reuses it).
GNU runs -Q, with `save-place-mode' as Romacs's defaults have it, and
tree-sitter modes (`treesit-enabled-modes' t, grammars
from --grammars: `tools/treesit_grammars.py so --out DIR') or, for
reference, its regex modes; Romacs runs its interactive defaults, whose
tree-sitter modes are the shipped grammars'.

usage: open_fontify_live.py --romacs BIN --grammars DIR --output OUT FILE...
       [--gnu EMACS] [--runs N] [--engines romacs,gnu-ts,gnu-regex] [--mode WxH]
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import live_oracle as live  # noqa: E402
import parity_live  # noqa: E402

MEASURE = r''';;; -*- lexical-binding: t -*-
(defvar ofl-out (getenv "OFL_OUT"))
;; A 15 MB file would otherwise stop at "File is large, really open?",
;; which a person answers before the measured visit.
(setq large-file-warning-threshold nil)
(defvar ofl-t0 nil)
(defvar ofl-command nil)
(defun ofl-log (format-string &rest args)
  (let ((line (concat (apply #'format format-string args) "\n"))
        (coding-system-for-write 'utf-8))
    (write-region line nil ofl-out t 'silent)))
(defun ofl-final ()
  "Whether the selected window's text all has its final faces: every
character `fontified', or a buffer jit-lock does not fontify."
  (let* ((start (window-start))
         (unfontified (lambda (end)
                        (with-current-buffer (window-buffer)
                          (text-property-any start (min end (point-max)) 'fontified nil)))))
    (with-current-buffer (window-buffer)
      (or (not (memq 'jit-lock-function fontification-functions))
          ;; The end the redisplay left costs nothing to poll; a timer may
          ;; have moved the start since, so the answer is the updated end's.
          ;; Updating it lays the window out in Romacs, once, not every poll.
          (and (not (funcall unfontified (window-end)))
               (not (funcall unfontified (window-end nil t))))))))
(defvar ofl-redisplays 0 "The redisplays GNU has begun.")
(unless (fboundp 'romacs--redisplay-since-mark)
  (add-function :before pre-redisplay-function
                (lambda (&rest _) (setq ofl-redisplays (1+ ofl-redisplays)))))
(defun ofl-mark ()
  "A function answering whether a redisplay ran since this call."
  (if (fboundp 'romacs--redisplay-since-mark)
      (progn (romacs--redisplay-since-mark t)
             (lambda () (romacs--redisplay-since-mark)))
    (let ((seen ofl-redisplays))
      (lambda () (> ofl-redisplays seen)))))
(defun ofl-pre () (setq ofl-t0 (float-time) ofl-command this-command))
(defvar ofl-poll nil "The timer waiting for the last command's final faces.")
(defun ofl-post ()
  "Poll every 2 ms until a redisplay has finished and the window's text
all has its final faces, which only a redisplay gives it (jit-lock), then
log the command's start, its end, that moment and how many polls it took."
  (let ((t0 ofl-t0) (command ofl-command) (post (float-time)) (polls 0)
        (redisplayed (ofl-mark)) timer)
    (when ofl-poll (cancel-timer ofl-poll))
    (setq timer
          (run-at-time
           0 0.002
           (lambda ()
             (setq polls (1+ polls))
             (let* ((now (float-time)) (final (and (funcall redisplayed) (ofl-final))))
               (when (or final (> (- (float-time) t0) 300))
                 (cancel-timer timer)
                 (when (eq ofl-poll timer) (setq ofl-poll nil))
                 (ofl-log "{\"event\":\"command\",\"command\":\"%s\",\"t0\":%.6f,\"post\":%.6f,\"t1\":%.6f,\"polls\":%d,\"buffer\":\"%s\",\"mode\":\"%s\",\"final\":%s,\"start\":%d,\"end\":%d}"
                          command t0 post now polls (buffer-name (window-buffer))
                          (with-current-buffer (window-buffer) major-mode)
                          (if final "true" "false") (window-start) (window-end nil t)))))))
    (setq ofl-poll timer)))
(add-hook 'pre-command-hook #'ofl-pre)
(add-hook 'post-command-hook #'ofl-post)
(defun ofl-fontify-all ()
  "Fontify the whole buffer, as `font-lock-ensure' does, and log the seconds."
  (interactive)
  (let ((t0 (float-time)))
    (font-lock-ensure)
    (ofl-log "{\"event\":\"fontify-all\",\"t0\":%.6f,\"seconds\":%.6f,\"chars\":%d}" t0 (- (float-time) t0) (buffer-size))))
(global-set-key [f9] #'ofl-fontify-all)
(run-with-idle-timer
 1 nil
 (lambda ()
   (ofl-log "{\"event\":\"ready\",\"pid\":%d,\"lines\":%d,\"columns\":%d}"
            (emacs-pid) (window-body-height) (window-body-width))))
'''

GNU_TS = '''(setq treesit-extra-load-path (list {grammars}))
(require 'treesit)
(customize-set-variable 'treesit-enabled-modes t)
'''


def events(path):
    """The JSON lines MEASURE wrote so far."""
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(errors='replace').splitlines() if line.startswith('{')]


def latency(stderr):
    """Romacs's --trace-latency records, in order."""
    found = []
    for line in stderr.read_text(errors='replace').splitlines() if stderr.exists() else []:
        if line.startswith('{') and '"latency-key"' in line:
            try:
                found.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return found


def rss(pid):
    """VmRSS and VmHWM of PID, in MiB."""
    fields = {}
    for line in Path(f'/proc/{pid}/status').read_text().splitlines():
        key, _, value = line.partition(':')
        if key in ('VmRSS', 'VmHWM'):
            fields[key] = round(int(value.split()[0]) / 1024, 1)
    return fields


def ret_render(traces, shown):
    """Romacs: RET to the observed end of the render that showed the file.
    The tracer counts C-x C-f and the minibuffer keys read inside it as one
    key, so this is that key's command end to render (`--trace-latency')
    plus RET's command start to command end (MEASURE)."""
    if not traces or not traces[-1].get('key_to_render_observed_ns'):
        return None
    last = traces[-1]
    after = (last['key_to_render_observed_ns'] - last['key_to_command_complete_ns']) / 1e9
    return round(after + shown['post'] - shown['t0'], 4)


class Session:
    """One editor in one private compositor, typed into with wtype."""

    def __init__(self, engine, args, root, file):
        self.engine, self.args, self.root, self.file = engine, args, root, file
        self.out = root / 'events.jsonl'
        self.stderr = root / 'editor.stderr'

    def wait(self, predicate, what, timeout=None):
        deadline = time.time() + (timeout or self.args.timeout)
        while time.time() < deadline:
            value = predicate()
            if value:
                return value
            if self.process.poll() is not None:
                raise RuntimeError(f'{self.engine} exited while waiting for {what}')
            time.sleep(0.02)
        raise TimeoutError(f'{self.engine}: {what}')

    def keys(self, *argv, gap=0.05):
        subprocess.run(['wtype', '-d', '20', *argv], env=self.env, check=True, timeout=30)
        time.sleep(gap)

    def save(self, name):
        """Where to keep the frames of NAME, with --keep-frames."""
        if not self.args.keep_frames:
            return None
        directory = self.args.output / 'frames'
        directory.mkdir(exist_ok=True)
        return str(directory / f'{self.engine}-{self.file.name}-{name}-{int(time.time())}')

    def command_events(self):
        return [e for e in events(self.out) if e['event'] == 'command']


    def start_compositor(self):
        """The private compositor, HOME and work directory, and the keyboard."""
        home, work, config = self.root / 'home', self.root / 'work', self.root / 'labwc'
        for directory in (home, work, config):
            directory.mkdir()
        shutil.copy(self.file, work / self.file.name)
        # Typing needs a writable visit, whatever the corpus file's mode.
        (work / self.file.name).chmod(0o644)
        (config / 'rc.xml').write_text(live.RC_XML)
        measure = self.root / 'ofl-measure.el'
        measure.write_text(MEASURE)
        run_root = Path(os.environ['XDG_RUNTIME_DIR']) / f'ofl{os.getpid()}-{int(time.time() * 1000) % 100000}'
        run_root.mkdir(mode=0o700)
        self.run_root = run_root
        env = {'PATH': os.environ['PATH'], 'HOME': str(home), 'LANG': 'C.UTF-8',
               'XDG_RUNTIME_DIR': str(run_root), 'XDG_CONFIG_HOME': str(home / '.config'),
               'XDG_CACHE_HOME': self.args.cache, 'XDG_DATA_HOME': str(home / '.local/share'),
               'XDG_STATE_HOME': str(home / '.local/state'), 'TMPDIR': str(self.root),
               'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
               'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop', 'OFL_OUT': str(self.out)}
        for item in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split():
            key, _, value = item.partition('=')
            env[key] = value
        for key in ('ROMACS_GNU_SOURCE', 'ROMACS_RUNTIME_ROOT', 'ROMACS_LISP_DIR'):
            if os.environ.get(key):
                env[key] = os.environ[key]
        self.env = env
        self.compositor_log = (self.root / 'compositor.log').open('w')
        self.compositor = subprocess.Popen([shutil.which('labwc', path=env['PATH']) or live.LABWC, '-C', str(config)],
                                           env=env, stdout=self.compositor_log, stderr=self.compositor_log)
        deadline = time.time() + 20
        while not any(p.is_socket() for p in run_root.glob('wayland-*')):
            if time.time() > deadline or self.compositor.poll() is not None:
                raise RuntimeError('compositor did not start')
            time.sleep(0.05)
        env['WAYLAND_DISPLAY'] = next(p.name for p in run_root.glob('wayland-*') if p.is_socket())
        subprocess.run(['wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', self.args.mode],
                       env=env, check=True, timeout=15)
        self.keyboard = subprocess.Popen(['wtype', '-s', '3600000', '-k', 'Shift_L'], env=env,
                                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(0.3)

    def start(self):
        self.start_compositor()
        env, home, measure = self.env, self.root / 'home', self.root / 'ofl-measure.el'
        work = self.root / 'work'
        if self.engine.startswith('romacs'):
            (home / '.config/romacs').mkdir(parents=True)
            (home / '.config/romacs/init.el').write_text(f'(load {json.dumps(str(measure))} nil t)\n')
            env.update(ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='14')
            argv = [self.args.romacs, '--loadup', '--trace-latency']
        else:
            setup = self.root / 'ofl-gnu.el'
            # Romacs's defaults restore a visited file's place, which a re-open
            # after scrolling then pays for: the same visit in both.
            body = '(setq inhibit-startup-screen t native-comp-jit-compilation nil)\n(save-place-mode 1)\n'
            if self.engine == 'gnu-ts':
                body += GNU_TS.format(grammars=json.dumps(self.args.grammars))
            setup.write_text(body + f'(load {json.dumps(str(measure))} nil t)\n')
            argv = [self.args.gnu, '-Q', '--font', 'Iosevka-14', '-l', str(setup)]
        self.stderr_file = self.stderr.open('w')
        self.process = subprocess.Popen(argv, env=env, cwd=work, stdin=subprocess.DEVNULL,
                                        stdout=self.stderr_file, stderr=self.stderr_file)
        ready = self.wait(lambda: next((e for e in events(self.out) if e['event'] == 'ready'), None), 'ready')
        self.pid, self.viewport = ready['pid'], (ready['lines'], ready['columns'])

    def stop(self):
        for proc in (getattr(self, 'process', None), getattr(self, 'keyboard', None), getattr(self, 'compositor', None)):
            live.stop(proc)
        try:
            self.run_root.rmdir()
        except (OSError, AttributeError):
            pass

    def open_file(self):
        """C-x C-f NAME RET: the open command's event once its window is
        fully fontified, the events up to it, and the Romacs RET latency."""
        before = len(self.command_events())
        traced = len(latency(self.stderr))
        self.keys('-M', 'ctrl', '-k', 'x', '-k', 'f', '-m', 'ctrl', gap=0.3)
        self.keys(self.file.name, gap=0.2)
        self.keys('-k', 'Return', gap=0)
        name = self.file.name

        def opened():
            new = self.command_events()[before:]
            shown = [e for e in new if e['buffer'] == name]
            return shown and shown
        shown = self.wait(opened, f'{name} shown')
        first_final = self.wait(lambda: next((e for e in self.command_events()[before:]
                                              if e['buffer'] == name and e['final']), None), f'{name} fontified')
        time.sleep(0.5)
        traces = latency(self.stderr)[traced:]
        return {'first_shown': shown[0], 'first_final': first_final, 'traces': traces[-3:],
                'first_paint_seconds': round(first_final['t1'] - shown[0]['t0'], 4),
                'command_seconds': round(shown[0]['post'] - shown[0]['t0'], 4),
                'polls': first_final['polls'],
                'mode': shown[0]['mode'],
                'romacs_ret_key_to_render_seconds': ret_render(traces, shown[0])}

    def key_latencies(self, argv_list, label):
        """Each key's command start to the end of its redisplay (MEASURE),
        and, for Romacs, its enqueue to observed render."""
        rows = []
        for argv in argv_list:
            before, traced = len(self.command_events()), len(latency(self.stderr))
            self.keys(*argv, gap=0)
            event = self.wait(lambda: len(self.command_events()) > before and self.command_events()[before], label)
            if not event['final']:
                raise RuntimeError(f'{label}: the window never got its final faces')
            time.sleep(0.25)
            traces = latency(self.stderr)[traced:]
            rows.append({'command': event['command'], 'seconds': round(event['t1'] - event['t0'], 4),
                         'polls': event['polls'],
                         'render_seconds': (round(traces[-1]['key_to_render_observed_ns'] / 1e9, 4)
                                            if traces and traces[-1].get('key_to_render_observed_ns') else None)})
        return rows

    def run(self):
        result = {'engine': self.engine, 'file': self.file.name}
        self.start()
        result['viewport'] = self.viewport
        result['open'] = self.open_file()
        result['rss_open'] = rss(self.pid)
        result['scroll'] = self.key_latencies([('-M', 'ctrl', '-k', 'v', '-m', 'ctrl')] * 5, 'C-v')
        # C-x k RET kills the unmodified buffer; then the same open again.
        before = len(self.command_events())
        self.keys('-M', 'ctrl', '-k', 'x', '-m', 'ctrl', '-k', 'k', gap=0.3)
        self.keys('-k', 'Return', gap=0)
        self.wait(lambda: any(e['buffer'] != self.file.name for e in self.command_events()[before:]), 'killed')
        time.sleep(0.3)
        result['reopen'] = self.open_file()
        result['end'] = self.key_latencies([('-M', 'alt', '-k', 'greater', '-m', 'alt')], 'M->')
        result['typing'] = self.key_latencies([(c,) for c in 'abcdefghij'], 'typing')
        before = len(events(self.out))
        self.keys('-k', 'F9', gap=0.05)
        sent = time.time()
        self.keys('-M', 'ctrl', '-k', 'n', '-m', 'ctrl', gap=0)
        fontify = self.wait(lambda: next((e for e in events(self.out)[before:] if e['event'] == 'fontify-all'), None),
                            'fontify-all', timeout=self.args.timeout * 3)
        blocked = self.wait(lambda: next((e for e in events(self.out)[before:]
                                          if e['event'] == 'command' and e['command'] == 'next-line'), None), 'C-n')
        result['fontify_all_seconds'] = round(fontify['seconds'], 4)
        during = sent < fontify['t0'] + fontify['seconds']
        result['key_during_fontify_seconds'] = round(blocked['t1'] - sent, 4) if during else None
        result['rss_fontified'] = rss(self.pid)
        return result


# Pixels: for an editor with no hook (Zed), and to calibrate the hook
# measure against what reaches the screen.

def grab(env):
    """The compositor output now, as PPM bytes, and when (full scale: grim scales on the CPU, eight times slower)."""
    started = time.time()
    data = subprocess.run(['grim', '-t', 'ppm', '-o', 'HEADLESS-1', '-'], env=env,
                          capture_output=True, check=True, timeout=10).stdout
    return started, data


def differing(a, b):
    """The fraction of differing bytes between two frames of one size."""
    if len(a) != len(b):
        return 1.0
    x = int.from_bytes(a, 'little') ^ int.from_bytes(b, 'little')
    return 1.0 - x.to_bytes(len(a), 'little').count(0) / len(a)


SAME = 0.002  # frames differing in fewer bytes are the same picture (a cursor)


def text_area(frame, top, bottom):
    """FRAME's rows TOP .. height-BOTTOM: the text a window shows, without
    title, tab and status bars or the echo area, which change on timers of
    their own."""
    header_end = 0
    for _ in range(3):  # P6, "W H", maxval: three header lines
        header_end = frame.index(b'\n', header_end) + 1
    width, height = map(int, frame[3:frame.index(b'\n', 3)].split()) if frame[2:3] == b'\n' else (0, 0)
    row = width * 3
    return frame[header_end + top * row: header_end + (height - bottom) * row]


def settle(env, origin, timeout, quiet=1.0, area=(0, 0), save=None, before=None):
    """Frames from now until the text area (AREA = rows cut at the top and
    bottom) stopped changing for QUIET seconds: the earliest frame whose
    text area already shows the settled picture, its delay from ORIGIN, the
    polling interval, the frame count, when that frame was taken and how
    long a quiet settled it.  A heuristic: what changes after QUIET is not seen.  A
    screen that never stays quiet is a timeout.  SAVE: write that frame and
    the settled one as SAVE-first.ppm and SAVE-final.ppm.  BEFORE, the frame
    before the key: a settled picture that is the same is an error (the key
    did nothing), not a zero."""
    cut = lambda frame: text_area(frame, *area)  # noqa: E731
    frames = [grab(env)]
    before = cut(before) if before else None
    stable_since = frames[0][0]
    deadline = time.time() + timeout
    while time.time() < deadline:
        frames.append(grab(env))
        if differing(cut(frames[-1][1]), cut(frames[-2][1])) >= SAME:
            stable_since = frames[-1][0]
        if frames[-1][0] - stable_since >= quiet:
            break
    else:
        raise TimeoutError(f'the screen did not stay unchanged for {quiet} s')
    final = cut(frames[-1][1])
    if before is not None and differing(before, final) < SAME:
        raise RuntimeError('the key changed nothing on screen')
    first, first_frame = next((t, data) for t, data in frames if differing(cut(data), final) < SAME)
    if save:
        Path(f'{save}-first.ppm').write_bytes(first_frame)
        Path(f'{save}-final.ppm').write_bytes(frames[-1][1])
    gaps = [b[0] - a[0] for a, b in zip(frames, frames[1:])]
    return {'seconds': round(first - origin, 4), 'interval': round(statistics.median(gaps), 4) if gaps else None,
            'frames': len(frames), 'first_at': first, 'quiet': quiet}


def session_pids(name, run_root):
    """The processes NAME (any, when None) whose XDG_RUNTIME_DIR is RUN_ROOT:
    this session's own."""
    found = []
    for pid in filter(str.isdigit, os.listdir('/proc')):
        try:
            if name is not None and Path(f'/proc/{pid}/comm').read_text().strip() != name:
                continue
            if f'XDG_RUNTIME_DIR={run_root}'.encode() in Path(f'/proc/{pid}/environ').read_bytes().split(b'\0'):
                found.append(int(pid))
        except OSError:
            continue
    return found


def process_rss(name, run_root):
    """VmRSS and VmHWM (MiB) of this session's process NAME."""
    pids = session_pids(name, run_root)
    return rss(pids[0]) if pids else {}


ZED_SETTINGS = {
    'telemetry': {'diagnostics': False, 'metrics': False}, 'auto_update': False,
    'enable_language_server': False, 'cursor_blink': False, 'show_edit_predictions': False,
    'buffer_font_family': 'Iosevka', 'buffer_font_size': 14, 'ui_font_size': 14,
    'restore_on_startup': 'none', 'git': {'inline_blame': {'enabled': False}},
    'inlay_hints': {'enabled': False}, 'show_call_status_icon': False,
    # Nothing is fetched: no extension, language server or update download.
    'auto_install_extensions': {'html': False}, 'proxy': 'http://127.0.0.1:9',
}


# Rows cut from the top and bottom of the output: what is not the text
# area (Zed's title and tab bars and status bar; Romacs's status bar and
# echo area).
ZED_AREA = (112, 34)
ROMACS_AREA = (0, 40)


class ZedSession(Session):
    """Zed in the same private compositor and HOME, measured by pixels:
    opened in a running Zed (`finder_open'), scrolled with pagedown and
    ctrl-end, re-opened after ctrl-w, and then launched anew with the file
    (from the process's start)."""

    def launch(self, *paths):
        data = self.root / 'zed-data'
        argv = ['zeditor', '--foreground', '--user-data-dir', str(data), *map(str, paths)]
        self.stderr_file = self.stderr.open('a')
        started = time.time()
        self.process = subprocess.Popen(argv, env=self.env, cwd=self.root / 'work', stdin=subprocess.DEVNULL,
                                        stdout=self.stderr_file, stderr=self.stderr_file)
        return started

    def start(self):
        Session.start_compositor(self)
        (self.root / 'home/.config/zed').mkdir(parents=True)
        (self.root / 'home/.config/zed/settings.json').write_text(json.dumps(ZED_SETTINGS))

    def stop(self):
        # Zed's own children too (it may start helpers): every process of
        # this session's runtime directory but the compositor and keyboard,
        # which Session.stop ends.
        mine = {getattr(self, 'compositor', None) and self.compositor.pid, getattr(self, 'keyboard', None) and self.keyboard.pid}
        for pid in session_pids(None, getattr(self, 'run_root', None)):
            if pid not in mine:
                try:
                    os.kill(pid, 15)
                except OSError:
                    pass
        Session.stop(self)

    def cli_open(self, path, name):
        """`zeditor -e PATH' into the running Zed, measured from the CLI's start."""
        before = grab(self.env)[1]
        origin = time.time()
        subprocess.Popen(['zeditor', '--user-data-dir', str(self.root / 'zed-data'), '-e', str(path)],
                         env=self.env, cwd=self.root / 'work', stdin=subprocess.DEVNULL,
                         stdout=self.stderr_file, stderr=self.stderr_file)
        return settle(self.env, origin, self.args.timeout, quiet=self.args.quiet, area=ZED_AREA, save=self.save(name),
                      before=before)

    def finder_open(self):
        """Open the file in the running Zed through its CLI (`zeditor -e',
        as emacsclient opens one in a running Emacs; keys sent to a fresh
        directory workspace did not reach its file finder): from the CLI's
        start, which counts its own start-up against Zed."""
        return self.cli_open(self.root / 'work' / self.file.name, 'open')

    def run(self):
        result = {'engine': 'zed', 'file': self.file.name, 'viewport': None}
        self.start()
        work = self.root / 'work'
        # A running Zed on the work directory, no file open yet: the first
        # open is ctrl-p NAME RET, parse included.
        started = self.launch(work)
        settle(self.env, started, self.args.timeout, quiet=3.0)
        # Zed asks whether to trust a new project ("Unrecognized Project");
        # its default, Trust and Continue, is Enter, as a person answers it
        # once.  The dialog takes the keyboard and dims the window.
        self.keys('-k', 'Return', gap=1.5)
        # The CLI's own cost: a three-line C file opened the same way first.
        tiny = self.root / 'work' / 'tiny.c'
        tiny.write_text('/* A tiny file. */\nint main (void) { return 0; }\n')
        baseline = self.cli_open(tiny, 'tiny')
        opened = self.finder_open()
        result['open'] = {'first_paint_seconds': opened['seconds'], 'mode': 'zed', 'pixels': opened,
                          'tiny_seconds': baseline['seconds'],
                          'net_seconds': round(opened['seconds'] - baseline['seconds'], 4)}
        result['rss_open'] = process_rss('zed-editor', self.run_root)
        result['scroll'] = []
        for _ in range(5):
            before, origin = grab(self.env)[1], time.time()
            self.keys('-k', 'Next', gap=0)
            result['scroll'].append(settle(self.env, origin, self.args.timeout, quiet=0.6, area=ZED_AREA,
                                           save=self.save('pagedown'), before=before))
        before, origin = grab(self.env)[1], time.time()
        self.keys('-M', 'ctrl', '-k', 'End', '-m', 'ctrl', gap=0)
        result['end'] = [settle(self.env, origin, self.args.timeout, quiet=0.6, area=ZED_AREA, before=before)]
        # Closing the tab keeps Zed's buffer; this is Zed's own re-open.
        self.keys('-M', 'ctrl', '-k', 'w', '-m', 'ctrl', gap=1.0)
        reopen = self.finder_open()
        result['reopen'] = {'first_paint_seconds': reopen['seconds'], 'pixels': reopen}
        result['rss_fontified'] = process_rss('zed-editor', self.run_root)
        # A new Zed launched with the file: from the process's start.
        for pid in session_pids('zed-editor', self.run_root):
            os.kill(pid, 15)
        live.stop(self.process)
        time.sleep(1.0)
        started = self.launch(work / self.file.name)
        result['launch'] = settle(self.env, started, self.args.timeout, area=ZED_AREA)
        result['typing'] = []
        result['fontify_all_seconds'] = result['key_during_fontify_seconds'] = None
        return result


class RomacsPixelSession(Session):
    """Romacs measured both ways in one session, to calibrate the pixel
    measure against the hook one: RET sent to the earliest frame showing
    the settled picture, beside RET's command start to final faces, the
    hook's evidence that the settled picture is the final one."""

    def run(self):
        result = {'engine': 'romacs-pixel', 'file': self.file.name}
        self.start()
        result['viewport'] = self.viewport
        for name in ('open', 'reopen'):
            if name == 'reopen':
                self.keys('-M', 'ctrl', '-k', 'x', '-m', 'ctrl', '-k', 'k', gap=0.3)
                self.keys('-k', 'Return', gap=1.0)
            before = len(self.command_events())
            self.keys('-M', 'ctrl', '-k', 'x', '-k', 'f', '-m', 'ctrl', gap=0.3)
            self.keys(self.file.name, gap=0.5)
            shown_before, origin = grab(self.env)[1], time.time()
            self.keys('-k', 'Return', gap=0)
            pixels = settle(self.env, origin, self.args.timeout, quiet=self.args.quiet, area=ROMACS_AREA, save=self.save(name),
                            before=shown_before)
            final = next(e for e in self.command_events()[before:] if e['buffer'] == self.file.name and e['final'])
            # A frame is stamped when its grab starts, up to an interval
            # before its pixels are read.
            if pixels['first_at'] + pixels['interval'] < final['t1']:
                raise RuntimeError('the settled picture came before the faces landed')
            result[name] = {'pixels': pixels, 'hook_seconds': round(final['t1'] - final['t0'], 4),
                            'ret_to_command_start': round(final['t0'] - origin, 4),
                            'first_paint_seconds': pixels['seconds'], 'mode': final['mode']}
        result['scroll'] = []
        for _ in range(3):
            origin = time.time()
            self.keys('-M', 'ctrl', '-k', 'v', '-m', 'ctrl', gap=0)
            result['scroll'].append(settle(self.env, origin, self.args.timeout, quiet=0.6))
        result['end'] = result['typing'] = []
        result['fontify_all_seconds'] = result['key_during_fontify_seconds'] = None
        result['rss_open'] = result['rss_fontified'] = rss(self.pid)
        return result


def summary(results):
    """Median and p90 of each measure per engine and file."""
    def stat(values):
        values = sorted(v for v in values if v is not None)
        if not values:
            return None
        p90 = values[min(len(values) - 1, int(round(0.9 * (len(values) - 1))))]
        return {'median': round(statistics.median(values), 4), 'p90': round(p90, 4), 'n': len(values)}
    table = {}
    for r in results:
        if 'error' in r:
            continue
        key = f"{r['file']} {r['engine']}"
        row = table.setdefault(key, {'mode': r['open'].get('mode'), 'viewport': r.get('viewport'), 'samples': []})
        row['samples'].append(r)
    for row in table.values():
        s = row.pop('samples')
        row.update({
            'open_first_paint': stat([r['open']['first_paint_seconds'] for r in s]),
            'open_romacs_ret_render': stat([r['open'].get('romacs_ret_key_to_render_seconds') for r in s]),
            'open_hook_same_session': stat([r['open'].get('hook_seconds') for r in s]),
            'launch_first_paint': stat([r.get('launch', {}).get('seconds') for r in s]),
            'pixel_interval': stat([r['open'].get('pixels', {}).get('interval') for r in s]),
            'open_polls': [r['open'].get('polls') for r in s],
            'reopen_first_paint': stat([r['reopen']['first_paint_seconds'] for r in s]),
            'reopen_romacs_ret_render': stat([r['reopen'].get('romacs_ret_key_to_render_seconds') for r in s]),
            'scroll_key': stat([k['seconds'] for r in s for k in r['scroll']]),
            'end_key': stat([k['seconds'] for r in s for k in r['end']]),
            'typing_key': stat([k['seconds'] for r in s for k in r['typing']]),
            'fontify_all': stat([r['fontify_all_seconds'] for r in s]),
            'key_during_fontify': stat([r['key_during_fontify_seconds'] for r in s]),
            'rss_open_mib': stat([r['rss_open'].get('VmRSS') for r in s]),
            'rss_fontified_mib': stat([r['rss_fontified'].get('VmRSS') for r in s]),
        })
    return table


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--romacs', required=True)
    parser.add_argument('--gnu', default=os.environ.get('ROMACS_GNU_ORACLE'))
    parser.add_argument('--grammars', required=True, help='libtree-sitter-*.so for GNU (tools/treesit_grammars.py so)')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--runs', type=int, default=5)
    parser.add_argument('--engines', default='romacs,gnu-ts,gnu-regex')
    parser.add_argument('--mode', default='1600x1000')
    parser.add_argument('--timeout', type=float, default=120)
    parser.add_argument('--quiet', type=float, default=1.0,
                        help='seconds the screen must stay unchanged before a pixel-measured open counts as settled')
    parser.add_argument('--no-compile-lisp', dest='compile_lisp', action='store_false',
                        help='use the compiled-Lisp store as it stands')
    parser.add_argument('--keep-frames', action='store_true', help='keep the pixel measures\' first and settled frames')
    parser.add_argument('files', nargs='+', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    args.cache = str(args.output / 'cache')
    os.makedirs(args.cache, exist_ok=True)
    engines = args.engines.split(',')
    if any(e.startswith('romacs') for e in engines):
        # The world a --loadup session restores, built once, as an installer does.
        env = dict(os.environ, XDG_CACHE_HOME=args.cache)
        print('world', parity_live.prepare_world(args.romacs, env, args.output), flush=True)
        # And the compiled Lisp, as an installer fills it: without it each
        # library's first load in a session reads its source.
        if args.compile_lisp:
            # The sessions' store, beside the binary, whatever this shell names.
            env.pop('ROMACS_COMPILED_LISP_STORE', None)
            compiled = subprocess.run([args.romacs, '--compile-lisp-tree', '--jobs', '4'], env=env, stdout=subprocess.DEVNULL)
            if compiled.returncode not in (0, 2):  # 2: some files load as source
                sys.exit(f'--compile-lisp-tree failed: {compiled.returncode}')
    results = []
    for run in range(args.runs):
        for file in args.files:
            for engine in engines:
                if engine == 'gnu-regex' and file.suffix == '.rs':
                    continue  # pinned GNU has no regex Rust mode
                if engine == 'gnu-ts' and file.suffix == '.el':
                    continue  # no tree-sitter mode: gnu-regex is the same session
                if engine == 'zed' and file.suffix == '.el':
                    results.append({'engine': 'zed', 'file': file.name, 'run': run, 'error': 'n/a: Zed has no Emacs Lisp mode'})
                    continue
                with tempfile.TemporaryDirectory(prefix=f'ofl-{engine}-', dir=args.output,
                                                 ignore_cleanup_errors=True) as temporary:
                    kind = {'zed': ZedSession, 'romacs-pixel': RomacsPixelSession}.get(engine, Session)
                    session = kind(engine, args, Path(temporary), file.resolve())
                    try:
                        result = session.run()
                    except Exception as error:  # noqa: BLE001
                        result = {'engine': engine, 'file': file.name, 'error': repr(error)}
                        kept = args.output / f'failed-{engine}-{file.name}-{run}'
                        kept.mkdir(exist_ok=True)
                        for name in ('events.jsonl', 'editor.stderr', 'compositor.log'):
                            if (Path(temporary) / name).exists():
                                shutil.copy(Path(temporary) / name, kept / name)
                    finally:
                        session.stop()
                result['run'] = run
                results.append(result)
                (args.output / 'results.jsonl').open('a').write(json.dumps(result) + '\n')
                print(json.dumps({k: result.get(k) for k in ('engine', 'file', 'run', 'error')} |
                                 {'open': result.get('open', {}).get('first_paint_seconds')}), flush=True)
    report = {'romacs': args.romacs, 'gnu': args.gnu, 'runs': args.runs, 'summary': summary(results)}
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    print(json.dumps(report['summary'], indent=1))


if __name__ == '__main__':
    main()
