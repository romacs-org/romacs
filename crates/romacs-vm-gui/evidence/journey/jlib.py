"""Real-compositor driver for the journey ladder.

Launches the GUI binary built from this checkout with exactly the environment
scripts/romacs gives it (ROMACS_LISP_DIR, launch font, RUST_MIN_STACK unset),
on Hyprland workspace 2 (HDMI-A-1) without stealing the owner's workspace,
drives it with real Wayland key events (wtype) and reads back:

  * stdout   -- the VM's own reply per executed command: the command symbol
               that ran, the signal it raised, the echo message. This is
               buffer/VM state, not pixels.
  * stderr   -- per-frame cursor publication (point_byte per window) and,
               with --trace-scene, every painted glyph with its row role.
  * grim     -- a compositor capture of the client, for corroboration only.

The only difference from scripts/romacs is that stdout is kept in a file
instead of /dev/null, so the VM's command replies can be read.
"""
import json, os, subprocess, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
ENV = os.environ.copy()
ENV.pop('LD_LIBRARY_PATH', None)
ENV.pop('RUST_MIN_STACK', None)
ENV['XDG_RUNTIME_DIR'] = '/run/user/1000'
ENV['ROMACS_DIAGNOSTICS'] = '1'  # the per-paint lines read here
ENV.setdefault('WAYLAND_DISPLAY', 'wayland-1')
ENV['HYPRLAND_INSTANCE_SIGNATURE'] = sorted(os.listdir('/run/user/1000/hypr'))[0]


def run(*a, check=True):
    p = subprocess.run(a, text=True, env=ENV, capture_output=True)
    if check and p.returncode:
        raise RuntimeError(f'{a} -> {p.returncode}: {p.stderr}')
    return p.stdout


def hy(*a):
    return run('hyprctl', *a)


def clients():
    return json.loads(hy('-j', 'clients'))


def active():
    return json.loads(hy('-j', 'activewindow'))


class Gui:
    def __init__(self, out, binary, mode='default', extra=()):
        self.out = Path(out); self.out.mkdir(parents=True, exist_ok=True)
        self.stdout = self.out / 'stdout.jsonl'
        self.log = self.out / 'stderr.log'
        self.original = active().get('address')
        before = {c['pid'] for c in clients()}
        flag = {'default': [], 'gnu-lisp': ['--gnu-lisp'], 'loadup': ['--loadup']}[mode]
        args = ['--trace-scene'] + flag + list(extra)
        envs = f'ROMACS_LISP_DIR={ROOT}/gnu-lisp ROMACS_GUI_FONT=Iosevka ROMACS_GUI_FONT_SIZE=14'
        cmd = (f'env -u RUST_MIN_STACK -u LD_LIBRARY_PATH {envs} {binary} {" ".join(args)} '
               f'>{self.stdout} 2>{self.log}')
        self.cmd = cmd
        hy('eval', f'hl.exec_cmd({json.dumps("sh -c " + json.dumps(cmd))}, {{workspace="2 silent"}})')
        deadline = time.monotonic() + 120
        own = []
        while time.monotonic() < deadline:
            own = [c for c in clients() if c['class'] == 'romacs' and c['pid'] not in before]
            if own: break
            time.sleep(0.2)
        if not own:
            raise RuntimeError('no romacs window appeared; log tail:\n' + self.tail())
        c = own[0]
        self.pid, self.addr = c['pid'], c['address']
        self.map_seconds = None
        # wait for the first ready reply
        t0 = time.monotonic()
        while time.monotonic() - t0 < 60:
            if self.replies() and self.replies()[0].get('ready'): break
            time.sleep(0.1)
        time.sleep(0.5)
        hy('dispatch', f'hl.dsp.window.fullscreen({{window="address:{self.addr}",mode="fullscreen",state="on"}})')
        hy('dispatch', f'hl.dsp.focus({{window="address:{self.addr}"}})')
        time.sleep(0.8)

    def tail(self, n=20):
        try: return '\n'.join(self.log.read_text(errors='replace').splitlines()[-n:])
        except FileNotFoundError: return '(no log)'

    def alive(self):
        return any(c['address'] == self.addr for c in clients())

    def safe(self):
        c = active()
        if c.get('address') != self.addr:
            # Focus moved (a notification, or the owner). Take it back at most
            # a few times per run, and say so in the record.
            self.refocus = getattr(self, 'refocus', 0) + 1
            if self.refocus > 3:
                raise RuntimeError(f'focus is not on the romacs client (is {c.get("class")!r}); refusing to type')
            time.sleep(1.0)
            hy('dispatch', f'hl.dsp.focus({{window="address:{self.addr}"}})')
            time.sleep(0.5)
            c = active()
            if c.get('address') != self.addr:
                raise RuntimeError(f'could not refocus romacs (is {c.get("class")!r}); refusing to type')
        return c

    def replies(self):
        try: lines = self.stdout.read_text(errors='replace').splitlines()
        except FileNotFoundError: return []
        out = []
        for l in lines:
            try: out.append(json.loads(l))
            except ValueError: out.append({'unparsed': l})
        return out

    def settle(self, quiet=0.6, limit=10.0):
        """Wait until stdout and stderr stop growing for `quiet` seconds."""
        t0 = time.monotonic(); last = None; since = time.monotonic()
        while time.monotonic() - t0 < limit:
            sig = (self.stdout.stat().st_size, self.log.stat().st_size)
            if sig != last: last, since = sig, time.monotonic()
            elif time.monotonic() - since >= quiet: return
            time.sleep(0.1)

    def key(self, k, mod=None):
        self.safe()
        a = ['wtype']
        mods = mod if isinstance(mod, (list, tuple)) else ([mod] if mod else [])
        for m in mods: a += ['-M', m]
        a += ['-k', k]
        for m in reversed(mods): a += ['-m', m]
        run(*a)
        time.sleep(0.12)

    def chord(self, spec):
        """Emacs-ish spec: 'C-x C-f', 'M-<', 'a', 'DEL', 'TAB', 'RET'."""
        names = {'DEL': 'BackSpace', 'TAB': 'Tab', 'RET': 'Return', 'SPC': 'space', '<': 'less',
                 '>': 'greater', '/': 'slash', ':': 'colon', 'ESC': 'Escape'}
        for part in spec.split(' '):
            mods = []
            while len(part) > 2 and part[1] == '-' and part[0] in 'CMS':
                mods.append({'C': 'ctrl', 'M': 'alt', 'S': 'shift'}[part[0]]); part = part[2:]
            k = names.get(part, part)
            self.key(k, mods)

    def text(self, s):
        self.safe()
        run('wtype', '-d', '8', s)
        time.sleep(0.2)

    def frames(self, kind):
        out = []
        for l in self.log.read_text(errors='replace').splitlines():
            if f'"{kind}"' in l:
                try: out.append(json.loads(l))
                except ValueError: pass
        return out

    def last_scene(self):
        s = self.frames('submitted-scene')
        return s[-1]['glyphs'] if s else []

    def last_points(self):
        c = self.frames('cursor-publication')
        return c[-1]['points'] if c else []

    def rows(self, role=None):
        rows = {}
        for g in self.last_scene():
            if role and g['role'] != role: continue
            key = (g['role'], g['clip']['x'] if isinstance(g['clip'], dict) else g['clip'][0], g['y'])
            rows.setdefault(key, []).append((g['x'], g['char']))
        return [[k[0], k[1], k[2], ''.join(c for _, c in sorted(v))] for k, v in sorted(rows.items(), key=lambda kv: (kv[0][1], kv[0][2]))]

    def shot(self, name):
        c = self.safe()
        x, y = c['at']; w, h = c['size']
        p = self.out / f'{name}.png'
        run('grim', '-g', f'{x},{y} {w}x{h}', str(p))
        return p

    def close(self):
        try:
            if self.original: hy('dispatch', f'hl.dsp.focus({{window="address:{self.original}"}})')
        except Exception as e:
            print('focus restore failed', e)
        # The Wayland client pid is the GUI binary itself (sh and env exec it);
        # never pkill by name: other lanes run the same binary.
        try: os.kill(self.pid, 15)
        except ProcessLookupError: pass
