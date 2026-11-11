#!/usr/bin/env python3
"""Standard scenes under each theme, for judging a theme by eye.

One fresh private headless labwc and one `romacs-vm-gui --loadup' per
theme (private HOME/XDG/TMPDIR; keys typed with wtype into that
compositor only), the theme loaded by the session's init.  Scenes: a
split with an Emacs Lisp buffer, an active region and line numbers beside
an empty buffer (`code'); a Rust file in its tree-sitter mode (`rust');
org headings, markup and a source block (`org'); help for `find-file' beside a diff (`help-diff'); Emacs Lisp
beside dired (`elisp-dired'); a message, which Romacs shows as a toast (`toast');
Customize's buttons (`custom');
the command panel open (`complete'); with --gnosis-db, gnosis-dashboard
over a copy of that database (`gnosis'); and,
when the system has notmuch and its Lisp, notmuch-hello, a search and a
thread of a synthetic store (`notmuch_fixture.py'; `hello', `search',
`show').  Each scene is one PNG per theme, and a sheet stacks every
theme's shot of a scene, top to bottom in --themes order
(SHEET-SCENE.png).

With --gnu EMACS, pinned GNU (`tools/lib/gnu_oracle.py' refuses any other)
shows the same scenes first, as GNU-NAME, under the reference Rum
configuration (crates/romacs-face/tests/gnu/rum-configured.el's inputs).

On a real GPU (ROMACS_HARNESS_WLR_ENV, KEY=VALUE ..., passed to the
compositor alone) Romacs's effects show; with pixman they are off.

usage: theme_scenes_live.py --binary PATH --output DIR
                            [--themes default,rum,...] [--gnu EMACS]
                            [--scale 1] [--mode 1600x1000] [--labwc PATH] [--wlr-randr PATH]
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

import live_oracle as live
import notmuch_fixture

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
INPUTS = ROOT / 'crates/romacs-face/tests/gnu/inputs'
SITE_LISP = Path('/usr/share/emacs/site-lisp')
ANCHOR = 1798000000

# The reference Rum configuration, as rum-configured.el captures it, for GNU.
GNU_RUM = '''
(custom-set-faces
 '(default ((t (:height 150 :family "Iosevka"))))
 '(variable-pitch ((t (:inherit default :family "Iosevka"))))
 '(italic ((t (:family "Iosevka" :slant italic)))))
(add-hook 'enable-theme-functions
          (lambda (&rest _)
            (let ((bg (face-background 'default)))
              (custom-set-faces
               `(fringe ((t :background ,bg :foreground ,bg)))
               `(window-divider ((t :background ,bg :foreground ,bg)))
               `(window-divider-first-pixel ((t :background ,bg :foreground ,bg)))
               `(window-divider-last-pixel ((t :background ,bg :foreground ,bg)))))))
(add-to-list 'load-path "%(inputs)s")
(require 'modus-themes)
(setq modus-themes-italic-constructs nil modus-themes-bold-constructs nil
      modus-themes-mixed-fonts nil modus-themes-variable-pitch-ui nil
      modus-themes-disable-other-themes t
      modus-themes-headings '((1 . (ultrabold 1.35)) (2 . (semibold 1.2))
                              (agenda-date . (1.2)) (agenda-structure . (variable-pitch light 1.8))
                              (t . (1.15))))
(setq omarchy-color-bg "#000000" omarchy-color-fg "#d4c5a0"
      omarchy-color-bright-black "#756b59" omarchy-color-accent "#d48a20")
(set-face-attribute 'default nil :background omarchy-color-bg :foreground omarchy-color-fg)
(load "%(inputs)s/rum-emacs.el" nil t)
(enable-theme 'rum)
(setq display-line-numbers-type 'relative)
(add-hook 'prog-mode-hook #'display-line-numbers-mode)
(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)
'''

# Each scene key writes its name to TS_LOG when it has run.
SCENES = r'''
(setq inhibit-startup-screen t)
(blink-cursor-mode -1)
(when (getenv "TS_NOTMUCH")
  (add-to-list 'load-path (getenv "TS_NOTMUCH"))
  (require 'notmuch)
  (setq notmuch-search-oldest-first nil))
(defun ts-done (name)
  (write-region (format "%S\n" name) nil (getenv "TS_LOG") t 'quiet))
(defun ts-when (ready then &optional tries)
  "Call THEN once READY returns non-nil, polling every 0.1 s, at most 30 s."
  (cond ((funcall ready) (funcall then))
        ((< (or tries 0) 300) (run-at-time 0.1 nil #'ts-when ready then (1+ (or tries 0))))))
(defun ts-search-done ()
  "Whether the selected notmuch search has finished with results."
  (and (derived-mode-p 'notmuch-search-mode)
       (not (get-buffer-process (current-buffer)))
       (> (buffer-size) 0)))
(defun ts-src () (find-file (getenv "TS_SRC")) (goto-char (point-min)))
(global-set-key (kbd "<f2>") (lambda () (interactive)
  (delete-other-windows) (ts-src) (forward-line 20) (set-window-start nil (point-min))
  (split-window-right)
  (with-selected-window (next-window) (switch-to-buffer (get-buffer-create "*notes*")))
  (ts-done 'code)))
(global-set-key (kbd "<f3>") (lambda () (interactive)
  (deactivate-mark) (delete-other-windows)
  (switch-to-buffer (get-buffer-create "headings.org")) (erase-buffer)
  (insert "* Level one heading\nBody text in the default face.\n** TODO Level two heading\n*** DONE Level three heading\n"
          "Some *bold*, /italic/, ~code~ and [[https://example.org][a link]].\n#+begin_src emacs-lisp\n(message \"hi\")\n#+end_src\n")
  (org-mode) (goto-char (point-min)) (ts-done 'org)))
(global-set-key (kbd "<f4>") (lambda () (interactive)
  (delete-other-windows) (describe-function 'find-file) (delete-other-windows) (switch-to-buffer "*Help*")
  (split-window-right)
  (with-selected-window (next-window)
    (switch-to-buffer (get-buffer-create "sample.diff")) (erase-buffer)
    (insert-file-contents (getenv "TS_DIFF")) (diff-mode) (goto-char (point-min)))
  (ts-done 'help-diff)))
(global-set-key (kbd "<f5>") (lambda () (interactive)
  (delete-other-windows) (ts-src) (re-search-forward "^(defun" nil t) (beginning-of-line)
  (set-window-start nil (point)) (split-window-right)
  (with-selected-window (next-window) (dired (file-name-directory (getenv "TS_SRC"))))
  (ts-done 'elisp-dired)))
(global-set-key (kbd "<f1>") (lambda () (interactive)
  (delete-other-windows) (find-file (getenv "TS_RUST")) (goto-char (point-min))
  (re-search-forward "^pub fn publish_face_text" nil t) (beginning-of-line)
  (set-window-start nil (point)) (ts-done 'rust)))
(global-set-key (kbd "<f6>") (lambda () (interactive) (delete-other-windows) (notmuch-hello) (ts-done 'hello)))
(global-set-key (kbd "<f7>") (lambda () (interactive)
  (delete-other-windows) (notmuch-search "tag:inbox")
  (ts-when #'ts-search-done (lambda () (ts-done 'search)))))
(global-set-key (kbd "<f8>") (lambda () (interactive)
  (goto-char (point-min)) (notmuch-search-show-thread) (delete-other-windows)
  (when (derived-mode-p 'notmuch-show-mode) (ts-done 'show))))
(global-set-key (kbd "<f10>") (lambda () (interactive)
  (message "Wrote %s (%d lines)" (abbreviate-file-name (getenv "TS_SRC")) 1234) (ts-done 'toast)))
(global-set-key (kbd "<f9>") (lambda () (interactive) (delete-other-windows) (switch-to-buffer "*scratch*") (ts-done 'scratch)))
(global-set-key (kbd "<f12>") (lambda () (interactive)
  (delete-other-windows) (customize-group 'faces) (delete-other-windows) (goto-char (point-min)) (ts-done 'custom)))
(when (getenv "TS_GNOSIS")
  (setq gnosis-dir (getenv "TS_GNOSIS"))
  (global-set-key (kbd "<f11>") (lambda () (interactive) (delete-other-windows) (gnosis-dashboard) (delete-other-windows) (ts-done 'gnosis))))
'''


def session(engine, binary, theme, args, out):
    """Every scene under THEME in one session of ENGINE; the PNGs' paths."""
    out.mkdir(parents=True, exist_ok=True)
    shots = {}
    with tempfile.TemporaryDirectory(prefix='theme-scenes-', dir=os.environ.get('XDG_RUNTIME_DIR')) as tmp:
        root = Path(tmp)
        root.chmod(0o700)
        home, config = root / 'home', root / 'config'
        for d in ('config', 'cache', 'data', 'state'):
            (home / d).mkdir(parents=True)
        config.mkdir()
        (config / 'rc.xml').write_text(live.RC_XML)
        src = root / 'src'
        src.mkdir()
        for name in ('romacs-defaults.el', 'romacs-theme.el', 'romacs-frame.el', 'romacs-chrome.el'):
            shutil.copy(ROOT / 'lisp/romacs' / name, src / name)
        diff = subprocess.run(['git', '-C', str(ROOT), 'log', '-1', '-p', '--format=', '--', 'lisp/romacs/romacs-theme.el'],
                              capture_output=True, text=True).stdout or ';; no diff\n'
        (root / 'sample.diff').write_text(diff)
        notmuch = shutil.which('notmuch') and (SITE_LISP / 'notmuch.el').exists()
        env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'HOME': str(home), 'LANG': 'C.UTF-8',
               'XDG_RUNTIME_DIR': str(root), 'XDG_CONFIG_HOME': str(home / 'config'),
               'XDG_CACHE_HOME': str(home / 'cache'), 'XDG_DATA_HOME': str(home / 'data'),
               'XDG_STATE_HOME': str(home / 'state'), 'TMPDIR': str(root),
               'WLR_BACKENDS': 'headless', 'WLR_RENDERER': 'pixman', 'WLR_HEADLESS_OUTPUTS': '1',
               'WLR_LIBINPUT_NO_DEVICES': '1', 'LIBSEAT_BACKEND': 'noop',
               'TS_LOG': str(root / 'scenes.txt'), 'TS_SRC': str(src / 'romacs-defaults.el'),
               'TS_DIFF': str(root / 'sample.diff'), 'TS_RUST': str(ROOT / 'crates/romacs-font/src/frame.rs')}
        if args.gnosis_db:
            (root / 'gnosis').mkdir()
            shutil.copy(args.gnosis_db, root / 'gnosis' / 'gnosis.db')
            (root / 'gnosis' / 'gnosis.db').chmod(0o644)
            env['TS_GNOSIS'] = str(root / 'gnosis') + '/'
        if notmuch:
            notmuch_fixture.build(root / 'store', ANCHOR)
            env.update(notmuch_fixture.environment(root / 'store'), TS_NOTMUCH=str(SITE_LISP))
        if engine == 'gnu':
            # The packages Romacs bundles (gnosis among them), on GNU's
            # `load-path' with Romacs's generated autoloads, as
            # gnosis_parity_live.py gives them.
            bundled = ' '.join(f'"{ROOT / "lisp/romacs" / d}"' for d in ('gnosis', 'emacs-lisp', '', 'deps/fsm', 'deps/markdown-mode', 'deps/websocket'))
            packages = (f'(setq load-path (append (list {bundled}) load-path))\n'
                        f'(load "{ROOT / "lisp/romacs/romacs-bundled-autoloads.el"}" nil t)\n') if args.gnosis_db else ''
            (home / 'init.el').write_text(GNU_RUM % {'inputs': INPUTS} + packages + SCENES)
            command = [str(binary), '-Q', '-l', str(home / 'init.el')]
            env['GDK_BACKEND'] = 'wayland'
        else:
            (home / 'config' / 'romacs').mkdir()
            (home / 'config' / 'romacs' / 'init.el').write_text(
                f"(load-theme '{theme} t)\n" + SCENES)
            command = [str(binary), '--loadup']
            env.update(ROMACS_GUI_FONT='Iosevka', ROMACS_GUI_FONT_SIZE='15', ROMACS_DIAGNOSTICS='1')
            for k in ('CARGO_HOME', 'ROMACS_RUNTIME_ROOT', 'ROMACS_LISP_DIR', 'ROMACS_WORLD'):
                if k in os.environ:
                    env[k] = os.environ[k]
        wlr = dict(pair.split('=', 1) for pair in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split())
        procs = []
        log_path = out / 'binary.stderr'
        try:
            with (out / 'compositor.log').open('w') as clog, log_path.open('w') as log:
                procs.append(subprocess.Popen([args.labwc, '-C', str(config)], env=env | wlr, stdout=clog, stderr=clog))
                wait(lambda: any(p.is_socket() for p in root.glob('wayland-*')), 15, 'compositor')
                env['WAYLAND_DISPLAY'] = next(p.name for p in root.glob('wayland-*') if p.is_socket())
                subprocess.run([args.wlr_randr, '--output', 'HEADLESS-1', '--custom-mode', args.mode,
                                '--scale', str(args.scale)], env=env, check=True, timeout=15)
                procs.append(subprocess.Popen(['/usr/bin/wtype', '-s', '600000', '-k', 'Shift_L'], env=env,
                                              stdout=log, stderr=log))
                time.sleep(.3)
                # GNU's GTK draws with the compositor's GL stack; Romacs uses the host's Vulkan.
                procs.append(subprocess.Popen(command, env=env | wlr if engine == 'gnu' else env, cwd=home,
                                              stdin=subprocess.DEVNULL, stdout=log, stderr=log))
                engine_proc = procs[-1]
                if engine == 'gnu':
                    time.sleep(8)
                else:
                    wait(lambda: '"diagnostic":"paint' in log_path.read_text(errors='replace'), args.timeout,
                         'first frame', engine_proc)
                    time.sleep(3)
                log_file = root / 'scenes.txt'

                def keys(*parts):
                    subprocess.run(['/usr/bin/wtype', '-d', '40', *parts], env=env, check=True, timeout=30)

                def scene(key, name, settle=1.5):
                    done = lambda: log_file.exists() and f'{name}\n' in log_file.read_text()
                    keys('-k', key)
                    wait(done, args.timeout, f'scene {name}', engine_proc)
                    time.sleep(settle)
                    path = out / f'{name}.png'
                    subprocess.run(['/usr/bin/grim', str(path)], env=env, check=True, timeout=30)
                    shots[name] = path

                keys('-k', 'F2')
                wait(lambda: log_file.exists() and 'code\n' in log_file.read_text(), args.timeout, 'scene code', engine_proc)
                time.sleep(1)
                keys('-M', 'ctrl', '-k', 'space', '-m', 'ctrl')
                for _ in range(6):
                    keys('-M', 'ctrl', 'n', '-m', 'ctrl')
                time.sleep(1.5)
                shots['code'] = out / 'code.png'
                subprocess.run(['/usr/bin/grim', str(shots['code'])], env=env, check=True, timeout=30)
                scene('F3', 'org', 6)
                scene('F4', 'help-diff', 2)
                scene('F5', 'elisp-dired', 2)
                scene('F1', 'rust', 3)
                if notmuch:
                    scene('F6', 'hello', 2.5)
                    scene('F7', 'search', 2.5)
                    scene('F8', 'show', 2.5)
                scene('F9', 'scratch', 1)
                scene('F10', 'toast', .6)
                scene('F12', 'custom', 2.5)
                if args.gnosis_db:
                    scene('F11', 'gnosis', 4)
                keys('-M', 'alt', 'x', '-m', 'alt')
                time.sleep(2)
                shots['complete'] = out / 'complete.png'
                subprocess.run(['/usr/bin/grim', str(shots['complete'])], env=env, check=True, timeout=30)
                keys('-M', 'ctrl', 'g', '-m', 'ctrl')
        finally:
            for p in reversed(procs):
                if p.poll() is None:
                    p.terminate()
                    try:
                        p.wait(5)
                    except subprocess.TimeoutExpired:
                        p.kill()
    return shots


def wait(predicate, timeout, what, proc=None):
    end = time.monotonic() + timeout
    while not predicate():
        if proc is not None and proc.poll() is not None:
            raise AssertionError(f'{what}: the engine exited ({proc.returncode})')
        if time.monotonic() > end:
            raise AssertionError(f'timed out waiting for {what}')
        time.sleep(.05)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--binary', type=Path, required=True)
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--themes', default='default,rum')
    ap.add_argument('--gnu', help='pinned GNU, to show the reference Rum configuration first')
    ap.add_argument('--gnosis-db', type=Path, help='a copy of a gnosis.db for the `gnosis\' (dashboard) scene; copied again per session')
    ap.add_argument('--scale', type=float, default=1)
    ap.add_argument('--mode', default='1600x1000')
    ap.add_argument('--timeout', type=float, default=600)
    ap.add_argument('--labwc', default=live.LABWC)
    ap.add_argument('--wlr-randr', default=live.RANDR)
    args = ap.parse_args()
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    runs = [(t, 'romacs', args.binary.resolve()) for t in args.themes.split(',') if t]
    if args.gnu:
        gnu = subprocess.run(['python3', str(ROOT / 'tools/lib/gnu_oracle.py'), 'path'], capture_output=True, text=True,
                             env=dict(os.environ, ROMACS_GNU_ORACLE=args.gnu), check=True).stdout.strip()
        runs.insert(0, ('gnu-rum', 'gnu', Path(gnu)))
    report = {'themes': [name for name, _, _ in runs], 'scenes': {}, 'failures': [],
              'compositor_renderer': dict(p.split('=', 1) for p in os.environ.get('ROMACS_HARNESS_WLR_ENV', '').split()).get('WLR_RENDERER', 'pixman')}
    taken = {}
    for name, engine, binary in runs:
        try:
            taken[name] = session(engine, binary, name, args, args.output / name)
        except (AssertionError, OSError, subprocess.SubprocessError) as error:
            report['failures'].append(f'{name}: {error!r}')
    scenes = sorted({scene for shots in taken.values() for scene in shots})
    magick = shutil.which('magick')
    for scene in scenes:
        paths = [str(taken[n][scene]) for n, _, _ in runs if n in taken and scene in taken[n]]
        report['scenes'][scene] = paths
        if magick and paths:
            subprocess.run([magick, *paths, '-append', str(args.output / f'SHEET-{scene}.png')], check=True, timeout=120)
    report['status'] = 'FAIL' if report['failures'] or not taken else 'PASS'
    (args.output / 'report.json').write_text(json.dumps(report, indent=1) + '\n')
    print(json.dumps({'status': report['status'], 'failures': report['failures'], 'scenes': scenes}))
    return 0 if report['status'] == 'PASS' else 1


if __name__ == '__main__':
    raise SystemExit(main())
