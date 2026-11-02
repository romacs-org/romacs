"""Session profiles for the bench, perfbench and the app parity harnesses.

Two profiles.  stock, the default: no one's init, only the stock gnu-lisp
and rlisp, on a generic project (bench/fixture.py); GNU runs with -Q and
loads the bench's endpoint with -l, Romacs runs --loadup with that endpoint
as its only init (its GUI has -Q but no -l).  init: a real init, described
by a profile config outside the repository.  Both give GNU the settings
Romacs's --loadup makes by design (no menu, tool or scroll bars,
scroll-conservatively 101, Romacs's completion defaults), as the parity
harnesses do.

The profile config is a JSON file given with --profile-config or
$ROMACS_BENCH_PROFILE.  Relative paths are relative to the file; `~' is
expanded.  Every key is optional:

  init            an Elisp file of the init's blocks, each wrapped in
                  (bench-block NAME BODY...), which records a signal in
                  `bench-init-errors' and goes on; @HERMES_URL@ in it
                  becomes hermes_url
  sources         [[DIR, GLOB], ...]: the packages the init loads, copied
                  read-only into one directory, byte-compiled by the pinned
                  GNU for GNU and loaded as source by Romacs
  projects        [DIR, ...]: the first Git repository is cloned as the
                  session's project
  org             an org file copied to ~/Documents
  gnosis_db       a copy of a gnosis database (never the live one)
  gnosis_db_repo  a Git repository whose only commit holds that copy
  live_gnosis_db  [PATH, ...]: live databases to refuse, besides gnosis's
                  default locations
  hermes_url      the hermes dashboard to attach to (hermes's default)
  hermes_sources  [[DIR, GLOB], ...]: hermes and its dependencies for
                  hermes_parity_live.py (default: the copies in rlisp/)

SAFETY, enforced here whatever the config says: a live gnosis database is
refused, and hermes never starts a dashboard: an init that sets a start
mode other than remote is refused, the start mode is set to remote before
and after the init, and every dashboard acquisition is made remote.
"""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

from parity_live import ROOT, lisp_copy, lisp_string

CONFIG_ENV = 'ROMACS_BENCH_PROFILE'
PROFILES = ('stock', 'init')
HERMES_URL = 'http://127.0.0.1:9119'  # hermes-dashboard-transport-url's default
# Where gnosis keeps its database by default (gnosis-dir under the user's
# Emacs directory).
DEFAULT_LIVE_GNOSIS = ('~/.config/emacs/gnosis/gnosis.db', '~/.emacs.d/gnosis/gnosis.db')
HERMES_SOURCES = [
    (ROOT / 'rlisp/hermes', '*.el'),
    (ROOT / 'rlisp/emacs-lisp', 'keymap-popup.el'),
    (ROOT / 'rlisp/deps/websocket', 'websocket.el'),
    (ROOT / 'rlisp/deps/markdown-mode', 'markdown-mode.el'),
]
EXCLUDE = ('-tests.el', '-test.el', 'compat-tests.el', '-pkg.el', '-autoloads.el')
START_MODE = re.compile(r"start-mode\s+'?:?([\w-]+)")


def config_path(given=None):
    """The profile config given, or named by $ROMACS_BENCH_PROFILE, or None."""
    path = given or os.environ.get(CONFIG_ENV)
    return Path(path).expanduser() if path else None


def load(given=None):
    """The profile config as a dict with paths resolved; {} without one."""
    path = config_path(given)
    if path is None:
        return {}
    raw = json.loads(path.read_text())
    base = path.resolve().parent

    def resolve(value):
        return (base / Path(value).expanduser()).resolve()
    out = dict(raw)
    for key in ('init', 'org', 'gnosis_db', 'gnosis_db_repo'):
        if raw.get(key):
            out[key] = resolve(raw[key])
    for key in ('sources', 'hermes_sources'):
        if key in raw:
            out[key] = [(resolve(directory), pattern) for directory, pattern in raw[key]]
    out['projects'] = [resolve(p) for p in raw.get('projects', [])]
    out['live_gnosis_db'] = [resolve(p) for p in raw.get('live_gnosis_db', [])]
    return out


def collect(pairs):
    return sorted(p for directory, pattern in pairs for p in Path(directory).glob(pattern)
                  if not p.name.endswith(EXCLUDE))


def sources(config):
    return collect(config.get('sources', []))


def hermes_sources(config):
    return collect(config.get('hermes_sources', HERMES_SOURCES))


def hermes_url(config):
    return config.get('hermes_url', HERMES_URL)


def default_project(config):
    return next((p for p in config.get('projects', []) if (p / '.git').exists()), None)


def refuse_live_gnosis(db, config):
    """DB resolved; ValueError when it is a live gnosis database."""
    db = Path(db).expanduser().resolve()
    xdg = Path(os.environ.get('XDG_CONFIG_HOME') or '~/.config').expanduser() / 'emacs/gnosis/gnosis.db'
    live = [Path(p).expanduser().resolve() for p in (*DEFAULT_LIVE_GNOSIS, xdg)] + list(config.get('live_gnosis_db', []))
    if db in live:
        raise ValueError(f'refusing the live gnosis database {db}; give a copy')
    return db


def profile_init(config):
    """The init's blocks, checked: no hermes start mode but remote."""
    if not config.get('init'):
        raise ValueError(f'profile init needs a profile config with "init" (--profile-config or ${CONFIG_ENV})')
    text = Path(config['init']).read_text().replace('@HERMES_URL@', hermes_url(config))
    modes = {m.group(1) for m in START_MODE.finditer(text)} - {'remote'}
    if modes:
        raise ValueError(f'refusing an init that may start hermes (start mode {", ".join(sorted(modes))})')
    return text


# Hermes attaches, never starts: before the init, after it, and on every
# acquisition, whatever a command asks for.
HERMES_REMOTE = '''(setq hermes-dashboard-transport-start-mode 'remote)
(defun bench--hermes-attach (args)
  "ARGS of a dashboard acquisition, made to attach."
  (plist-put (copy-sequence args) :start-mode 'remote))
(with-eval-after-load 'hermes-dashboard-transport
  (setq hermes-dashboard-transport-start-mode 'remote)
  (advice-add 'hermes-dashboard-transport-acquire :filter-args #'bench--hermes-attach))
'''


def gnu_settings():
    """The settings Romacs's --loadup makes before the user's init, for
    GNU's init, as the parity harnesses' GNU takes them."""
    return ('(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
            '(setq scroll-conservatively 101)\n'
            f'(load {lisp_string(str(ROOT / "rlisp/romacs-completion-defaults.el"))} nil t)\n')


# GNU's completion as it loads rlisp/romacs-completion-defaults.el, for
# Romacs in a comparison: Romacs's window-system display is its own layer
# (romacs-complete-mode, DIVERGENCES.md VM2-MINI-COMPLETION-LAYER), which
# takes the root window's last line, and its styles begin with fzf
# (VM2-FZF-DEFAULT), which fido then completes with instead of flex; GNU
# gets `fido-vertical-mode' and the same styles without fzf, and no
# project-file override, which the defaults add only with fzf.
GNU_COMPLETION_UI = '''(romacs-complete-mode -1)
(setq completion-styles (remq 'fzf completion-styles)
      completion-category-overrides
      (mapcar (lambda (entry)
                (cons (car entry)
                      (mapcar (lambda (prop)
                                (if (eq (car-safe prop) 'styles) (cons 'styles (remq 'fzf (cdr prop))) prop))
                              (cdr entry))))
              (assq-delete-all 'project-file (copy-sequence completion-category-overrides))))
(fido-vertical-mode 1)
'''


def init_source(engine, lisp, port_file, token, profile, config):
    """ENGINE's init: under PROFILE init the config's blocks from LISP, under
    stock nothing of anyone's, then the MCP endpoint on loopback, its port
    written to PORT_FILE, its peers sending TOKEN first.  Loopback, not a
    local socket: Romacs's GUI host refuses local network processes."""
    real = profile == 'init'
    blocks = profile_init(config) if real else ''
    setup = gnu_settings() if engine == 'gnu' else GNU_COMPLETION_UI
    return f''';; -*- lexical-binding: t -*-
(defvar bench-init-errors nil "Init blocks that signalled, as (NAME MESSAGE).")
(defmacro bench-block (name &rest body)
  "Run BODY; record a signal under NAME and go on with the init."
  `(condition-case err (progn ,@body)
     (error (push (list ,name (error-message-string err)) bench-init-errors)
            (message "bench init: %s: %s" ,name (error-message-string err)))))
{f"(setq load-path (cons {lisp_string(str(lisp))} load-path))" if real else ""}
{setup}(setq inhibit-startup-screen t)
{HERMES_REMOTE if real else ""}{blocks}
{"(setq hermes-dashboard-transport-start-mode 'remote)" if real else ""}
(defun bench--strings (value)
  (cond ((stringp value) (list value))
        ((consp value) (append (bench--strings (car value)) (bench--strings (cdr value))))))
(defun bench-secrets ()
  "The credentials hermes holds for its dashboard."
  (let ((auth (bound-and-true-p hermes-dashboard-transport--api-auth)))
    (append (bench--strings (plist-get auth :discovered-token))
            (bench--strings (plist-get auth :secrets))
            (bench--strings (bound-and-true-p hermes-dashboard-transport--native-token-memory)))))
(load {lisp_string(str(ROOT / 'rlisp/romacs-mcp.el'))} nil t)
(load {lisp_string(str(ROOT / 'rlisp/romacs-mcp-session.el'))} nil t)
(setq romacs-mcp-session-secrets-function #'bench-secrets)
(romacs-mcp-enable-session-tools)
(cd "~/")
;; The endpoint starts here, not from `emacs-startup-hook': a session whose
;; startup fails after the init still answers, and says whether the hook ran.
(defvar bench-startup-hook-ran nil)
(add-hook 'emacs-startup-hook (lambda () (setq bench-startup-hook-ran t)) 90)
(romacs-mcp-start-loopback {lisp_string(token)} {lisp_string(str(port_file))})
(with-temp-file (expand-file-name "alive" (getenv "PARITY_PROBE_DIR"))
  (insert (number-to-string (float-time))))
'''


def git(*args, env):
    subprocess.run(['git', *args], env=env, check=True, capture_output=True, timeout=300)


def populate(home, project, org, gnosis_db, config):
    """Copy the session's data into HOME; return what was used.  The
    project a `git clone --no-hardlinks', so vc has history; the org file a
    copy under ~/Documents; the gnosis database a reflink copy of a copy
    (never the live one) in a fresh Git repository with no remote, at
    ~/gnosis."""
    env = {'HOME': str(home), 'PATH': os.environ['PATH'], 'GIT_CONFIG_NOSYSTEM': '1'}
    # Commits a session makes (gnosis's) land in the private HOME, unsigned.
    (home / '.gitconfig').write_text('[user]\n\tname = bench\n\temail = bench@localhost\n'
                                     '[commit]\n\tgpgsign = false\n[init]\n\tdefaultBranch = master\n')
    used = {}
    if project:
        target = home / 'Projects' / project.name
        target.parent.mkdir(parents=True, exist_ok=True)
        git('clone', '-q', '--no-hardlinks', str(project), str(target), env=env)
        used['project'] = f'~/Projects/{project.name}'
    if org:
        (home / 'Documents').mkdir(exist_ok=True)
        shutil.copyfile(org, home / 'Documents' / org.name)
        used['org'] = f'~/Documents/{org.name}'
    if gnosis_db:
        gnosis_db = refuse_live_gnosis(gnosis_db, config)
        target = home / 'gnosis'
        target.mkdir(parents=True)
        subprocess.run(['cp', '--reflink=auto', str(gnosis_db), str(target / 'gnosis.db')], check=True)
        (target / 'gnosis.db').chmod(0o644)
        for extra in ('images', 'assets'):
            if (gnosis_db.parent / extra).is_dir():
                subprocess.run(['cp', '-r', '--reflink=auto', str(gnosis_db.parent / extra), str(target / extra)],
                               check=True)
        git('init', '-q', str(target), env=env)
        git('-C', str(target), 'add', 'gnosis.db', env=env)
        git('-C', str(target), 'commit', '-q', '-m', 'Database copy', env=env)
        used['gnosis'] = '~/gnosis/gnosis.db'
    return used


def lisp_dir(base, engine, config):
    """The profile's package sources copied once under BASE/lisp (a
    directory of its own: a layer's plan lists a package directory's
    siblings); GNU's compiled."""
    (Path(base) / 'lisp').mkdir(exist_ok=True)
    return lisp_copy(Path(base) / 'lisp', f'init-{engine}', sources(config), engine == 'gnu')
