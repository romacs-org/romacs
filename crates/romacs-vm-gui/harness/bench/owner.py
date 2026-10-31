"""The sessions the bench starts: their init and data copies.

Two profiles.  stock, the default: no one's init, only the stock gnu-lisp
and rlisp, on a generic project (fixture.py); GNU runs with -Q and loads
the bench's endpoint with -l, Romacs runs --loadup with that endpoint as
its only init (its GUI has -Q but no -l).  owner: the owner's session,
below.  Both give GNU the settings Romacs's --loadup makes by design
(no menu, tool or scroll bars, scroll-conservatively 101, Romacs's
completion defaults), as the parity harnesses do.

The owner profile:

The init mirrors ~/.config/emacs/init.el's blocks that shape daily editing,
completion, org, dired, project, vc, eshell, gnosis and hermes, rewritten
without straight (which would fetch and build): each package's sources are
copied from the owner's checkouts and straight repos, read-only, into one
directory, byte-compiled by the pinned GNU for GNU and loaded as source by
Romacs, as hermes_parity_live.py does.  Not mirrored: theme, fonts and
frame alpha, doom-modeline, org-modern, flycheck, eglot, the mail, jabber
and agent packages, and the eshell aliases.  Each block runs under
`bench-block', which records a signal in `bench-init-errors' rather than
abandoning the rest of the init; bench_start reports them.

Data, each a copy in the engine's private HOME: the project a
`git clone --no-hardlinks' of a real repository, so vc has history; the org
file a copy of the one given (--org, an owner's file under ~/Documents); the gnosis database a reflink
copy of a database copy given explicitly (never the live one) in a fresh
Git repository with no remote, at ~/gnosis (the GNU engine's
~/.config/emacs holds its init).  Hermes uses the running dashboard at
127.0.0.1:9119, attached, never started.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import parity_live  # noqa: E402
from parity_live import ROOT, lisp_string  # noqa: E402

HOME = Path.home()
DASHBOARD_URL = 'http://127.0.0.1:9119'
LIVE_GNOSIS_DB = HOME / '.config/emacs/gnosis/gnosis.db'
STRAIGHT = HOME / '.config/emacs/straight/repos'
SOURCES = [
    (HOME / 'Projects/emacs-lisp/gnosis/lisp', '*.el'),
    (HOME / 'Projects/emacs-lisp/emacs-hermes/lisp', '*.el'),
    (HOME / 'Projects/emacs-lisp/keymap-popup', 'keymap-popup.el'),
    (HOME / 'Projects/emacs-lisp/vc-plus/lisp', '*.el'),
    (STRAIGHT / 'emacs-websocket', 'websocket.el'),
    (STRAIGHT / 'markdown-mode', 'markdown-mode.el'),
    (STRAIGHT / 'compat', 'compat*.el'),
    (STRAIGHT / 'eat', 'eat.el'),
]
EXCLUDE = ('-tests.el', '-test.el', 'compat-tests.el', '-pkg.el', '-autoloads.el')
PROJECTS = [HOME / 'Projects/emacs-lisp/emacs-hermes', HOME / 'Projects/rust/romacs']


def sources():
    return sorted(p for directory, pattern in SOURCES for p in directory.glob(pattern)
                  if not p.name.endswith(EXCLUDE))


def default_project():
    return next((p for p in PROJECTS if (p / '.git').exists()), None)


# The owner's blocks, as init.el has them, minus straight and host checks.
OWNER_INIT = r'''
(bench-block "emacs"
  (setf backup-directory-alist '((".*" . "~/.Trash"))
        sentence-end-double-space t
        gc-cons-threshold (* 128 1024 1024)
        disabled-command-function nil
        tab-always-indent 'icomplete-force-complete
        require-final-newline t
        read-extended-command-predicate #'command-completion-default-include-p
        visible-bell nil
        display-line-numbers-type 'relative
        inhibit-startup-message t
        initial-scratch-message nil
        read-process-output-max (* 5 1024 1024)
        save-interprogram-paste-before-kill t
        kill-do-not-save-duplicates t
        search-default-mode #'char-fold-to-regexp)
  (column-number-mode)
  (global-display-line-numbers-mode 1)
  (require 'savehist) (savehist-mode)
  (require 'saveplace) (save-place-mode 1)
  (require 'recentf) (recentf-mode 1)
  (electric-pair-mode 1)
  (delete-selection-mode 1)
  (tool-bar-mode -1) (tooltip-mode -1) (menu-bar-mode -1) (blink-cursor-mode -1)
  (scroll-bar-mode -1)
  (set-fringe-mode 10)
  (setq-default mode-line-format
                '("%e" mode-line-front-space
                  (:propertize
                   ("" mode-line-mule-info mode-line-client mode-line-modified
                    mode-line-remote mode-line-window-dedicated)
                   display (min-width (6.0)))
                  mode-line-frame-identification
                  mode-line-buffer-identification
                  "   "
                  mode-line-position
                  (project-mode-line project-mode-line-format)
                  (vc-mode vc-mode)
                  "  "
                  mode-line-modes
                  mode-line-misc-info
                  mode-line-end-spaces))
  (keymap-global-set "M-<backspace>" #'backward-kill-sexp)
  (keymap-global-set "C-c L" #'display-line-numbers-mode)
  (keymap-global-unset "C-z")
  (keymap-global-set "C-c r" #'rename-visited-file)
  (keymap-global-set "C-x 7" #'window-swap-states)
  (keymap-global-set "C-x r d" #'bookmark-delete)
  (keymap-global-set "C-x C-b" #'switch-to-prev-buffer)
  (keymap-global-set "C-x M-b" #'ibuffer)
  (keymap-global-set "C-c s" #'async-shell-command)
  (add-hook 'emacs-lisp-mode-hook #'prettify-symbols-mode)
  (add-hook 'lisp-mode-hook #'prettify-symbols-mode)
  (add-hook 'before-save-hook #'delete-trailing-whitespace))

(bench-block "completion"
  (require 'completion-preview)
  (global-completion-preview-mode)
  (add-hook 'org-mode-hook (lambda () (completion-preview-mode -1)))
  (require 'icomplete)
  (setf completion-styles '(basic flex)
        completion-auto-select t
        completion-auto-help 'visible
        completions-format 'one-column
        completions-sort 'historical
        completions-max-height 20
        completion-ignore-case t
        icomplete-compute-delay 0.15
        icomplete-delay-completions-threshold 400
        completion-category-overrides '((file (styles partial-completion))))
  (fido-vertical-mode 1)
  (keymap-set icomplete-fido-mode-map "TAB" #'icomplete-force-complete)
  (setf imenu-auto-rescan t imenu-flatten 'prefix)
  (which-key-mode 1))

(bench-block "search"
  (defun switch-to-window-with-buffer (buffer-name)
    "Switch to a window displaying a buffer named BUFFER-NAME, if it exists."
    (let ((target-window nil))
      (walk-windows (lambda (w)
                      (when (string= (buffer-name (window-buffer w)) buffer-name)
                        (setq target-window w)))
                    nil t)
      (if target-window
          (select-window target-window)
        (message "No window is displaying buffer '%s'" buffer-name))))
  (defun thanos/close-window (buffer-name)
    (let ((occur-window (get-buffer-window buffer-name)))
      (when occur-window (quit-window nil occur-window))))
  (defun thanos/occur (regexp &optional nlines region)
    "Run `occur' and switch to *Occur* buffer."
    (interactive (nconc (occur-read-primary-args)
                        (and (use-region-p) (list (region-bounds)))))
    (let* ((start (and (caar region) (max (caar region) (point-min))))
           (end (and (cdar region) (min (cdar region) (point-max))))
           (in-region (or start end))
           (bufs (if (not in-region) (list (current-buffer))
                   (let ((ol (make-overlay (or start (point-min)) (or end (point-max)))))
                     (overlay-put ol 'occur--orig-point (point))
                     (list ol)))))
      (occur-1 regexp nlines bufs))
    (switch-to-window-with-buffer "*Occur*")
    (forward-line))
  (defun thanos/occur-mode-goto-occurrence-close-occur-window ()
    (interactive)
    (occur-mode-goto-occurrence)
    (thanos/close-window "*Occur*"))
  (keymap-global-set "M-s M-o" #'thanos/occur)
  (with-eval-after-load 'replace
    (keymap-set occur-mode-map "RET" #'thanos/occur-mode-goto-occurrence-close-occur-window))
  (defun thanos/compile-goto-error ()
    (interactive (list last-input-event))
    (compile-goto-error)
    (thanos/close-window "*grep*"))
  (require 'grep)
  (keymap-global-set "M-s g" #'grep-find)
  (keymap-set grep-mode-map "C-o" #'thanos/compile-goto-error)
  (keymap-global-set "M-s M-f" #'find-name-dired))

(bench-block "markdown" (setq markdown-hide-urls nil))

(bench-block "org"
  (require 'org)
  (setf org-directory "~/Documents"
        org-agenda-files (let ((agenda-dir (expand-file-name "agenda" org-directory)))
                           (when (file-directory-p agenda-dir)
                             (directory-files-recursively agenda-dir "\.org\'")))
        org-default-notes-file (expand-file-name "notes.org" org-directory)
        org-ellipsis " ▼"
        org-log-done 'time
        org-hide-emphasis-markers nil
        org-table-convert-region-max-lines 20000
        org-startup-folded 'content
        org-todo-keywords '((type "TODO" "SEMINAR" "TEST" "MEETING" "|" "DONE" "FAILED"))
        org-todo-keyword-faces
        `(("FAILED" . (:inherit error))
          ("MEETING" . (:inherit font-lock-constant-face))
          ("SEMINAR" . (:inherit match))
          ("DONE" . (:inherit shadow))))
  (setq org-time-stamp-custom-formats '("<%Y-%m-%d>" . "<%Y-%m-%d %a %H:%M>"))
  (setq org-display-custom-times t)
  (setf org-imenu-depth 3)
  (setf org-structure-template-alist
        '(("e" . "src emacs-lisp") ("p" . "src python") ("l" . "src lisp")
          ("b" . "src bash") ("q" . "QUOTE")))
  (require 'ox-latex)
  (push 'org-self-insert-command completion-preview-commands)
  (add-hook 'org-mode-hook (lambda () (display-line-numbers-mode -1)))
  (keymap-global-set "C-c l" #'org-store-link)
  (keymap-set org-mode-map "C-c M-t" #'org-todo)
  (keymap-set org-mode-map "C-c RET" #'org-table-hline-and-move)
  (keymap-set org-mode-map "C-M-i" #'completion-at-point))

(bench-block "dired"
  (require 'dired)
  (keymap-set dired-mode-map "b" #'dired-up-directory)
  (keymap-set dired-mode-map "z" #'wdired-change-to-wdired-mode)
  (keymap-set dired-mode-map "C-c r" #'dired-do-query-replace-regexp))

(bench-block "project"
  (require 'project)
  (defun project-prefixed-buffer-name (mode)
    (let* ((project (project-current nil))
           (root (and project (project-root project))))
      (concat "*"
              (and root (file-remote-p root))
              (if project
                  (project-name project)
                (file-name-nondirectory (directory-file-name default-directory)))
              "-"
              (downcase mode)
              "*"))))

(bench-block "vc"
  (require 'vc-git)
  (setopt vc-git-root-log-format
          '("%d%h..: %an %ad %s"
            "^\\(?:[*/\\| .-]+ \\)?\\(?2: ([^)]+)\\)?\\(?1:[0-9a-z]+\\)\\.\\.: \\(?3:.*?\\)[ \t]+\\(?4:[0-9]\\{4\\}-[0-9]\\{2\\}-[0-9]\\{2\\}\\)"
            ((1 'log-view-message)
             (2 'change-log-list nil lax)
             (3 'change-log-name)
             (4 'change-log-date))))
  (setq vc-git-log-switches '("--show-signature" "--first-parent")
        vc-dir-show-key-binding-hints nil))

(bench-block "vc-plus" (require 'vc-plus) (vc-plus-mode))

(bench-block "eat"
  (require 'eat)
  (add-hook 'eshell-mode-hook #'eat-eshell-mode)
  (add-hook 'eshell-mode-hook #'eat-eshell-visual-command-mode))

(bench-block "eshell"
  (require 'esh-mode)
  (require 'eshell)
  (defun eshell-git-info ()
    "Return a string with git info."
    (when (eq (call-process "git" nil nil nil "rev-parse" "--is-inside-work-tree") 0)
      (let* ((branch-raw (shell-command-to-string "git rev-parse --abbrev-ref HEAD"))
             (branch (if (or (string-match-p "^fatal" branch-raw)
                             (string-match-p "^error" branch-raw))
                         "Unknown"
                       (string-trim branch-raw)))
             (dirty (not (string= "" (string-trim (shell-command-to-string "git status --porcelain")))))
             (dirty-info (if dirty " ✎" " ✔")))
        (concat (propertize "⎇ " 'face 'font-lock-type-face)
                (propertize branch 'face 'font-lock-keyword-face)
                (propertize dirty-info 'face (if dirty 'error 'success))))))
  (defface my/eshell-separator
    '((((background light)) :strike-through "gray70" :foreground "gray70")
      (t :strike-through "gray30" :foreground "gray30"))
    "Face for separator line in eshell.")
  (defun eshell-prompt-multiline ()
    "Eshell Multiline Git prompt."
    (let ((separator (propertize " | " 'face 'font-lock-comment-face))
          (hr (propertize (concat (make-string (/ (window-total-width) 2) ?─) "\n") 'face 'my/eshell-separator))
          (dir (propertize (format "%s" (abbreviate-file-name (eshell/pwd))) 'face 'font-lock-warning-face))
          (git-info (eshell-git-info))
          (time (propertize (format-time-string "%H:%M:%S") 'face 'font-lock-comment-face))
          (sign (if (= (user-uid) 0)
                    (propertize "\n#" 'face 'font-lock-constant-face)
                  (propertize "\nλ" 'face 'font-lock-negation-char-face))))
      (concat hr dir separator git-info separator time sign " ")))
  (setf eshell-prompt-function #'eshell-prompt-multiline
        eshell-highlight-prompt nil)
  (keymap-global-set "C-c e" #'eshell)
  (add-hook 'eshell-mode-hook (lambda () (display-line-numbers-mode -1) (eshell-cmpl-mode -1))))

(bench-block "gnosis"
  (require 'auth-source-pass)
  (auth-source-pass-enable)
  (define-prefix-command 'thanos/gnosis-map)
  (define-prefix-command 'thanos/notes)
  (define-prefix-command 'thanos/journal)
  (setq gnosis-dir (expand-file-name "~/gnosis/"))
  (setopt gnosis-model-renderer-directory (expand-file-name "~/Projects/emacs-lisp/gnosis/optional/canvas-3d")
          gnosis-script-input-method-alist '((greek . "greek-polytonic") (cyrillic . "bulgarian-phonetic"))
          gnosis-journal-bullet-point-char "-"
          gnosis-agent-eval-hermes-profile "grunt")
  (autoload 'gnosis "gnosis" nil t)
  (autoload 'gnosis-review "gnosis" nil t)
  (autoload 'gnosis-dashboard "gnosis-dashboard" nil t)
  (keymap-global-set "C-c c" 'thanos/gnosis-map)
  (keymap-global-set "C-x g" #'gnosis)
  (keymap-global-set "C-c n" 'thanos/notes)
  (keymap-global-set "C-c j" 'thanos/journal)
  (keymap-set thanos/gnosis-map "r" #'gnosis-review)
  (keymap-set thanos/notes "f" #'gnosis-nodes-find))

(bench-block "hermes"
  (define-prefix-command 'thanos/hermes)
  (setq hermes-instances '(("local" . "@URL@"))
        hermes-dashboard-transport-url "@URL@"
        hermes-dashboard-transport-start-mode 'remote)
  (keymap-global-set "C-M-;" 'thanos/hermes)
  (keymap-set thanos/hermes "h" #'hermes)
  (keymap-set thanos/hermes "c" #'hermes-chat)
  (keymap-set thanos/hermes "l" #'hermes-switch-to-chat)
  (keymap-set project-prefix-map "h" #'hermes-project-chat)
  (require 'hermes))
'''.replace('@URL@', DASHBOARD_URL)


def gnu_settings():
    """The settings Romacs's --loadup makes before the user's init, for
    GNU's init, as the parity harnesses' GNU takes them."""
    return ('(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
            '(setq scroll-conservatively 101)\n'
            f'(load {lisp_string(str(ROOT / "rlisp/romacs-completion-defaults.el"))} nil t)\n')


def init_source(engine, lisp, port_file, token, profile='owner'):
    """ENGINE's init: under PROFILE owner the owner's blocks from LISP, under
    stock nothing of anyone's, then the MCP endpoint on loopback, its port
    written to PORT_FILE, its peers sending TOKEN first.  Loopback, not a
    local socket: Romacs's GUI host refuses local network processes."""
    owner = profile == 'owner'
    gnu_setup = gnu_settings() if engine == 'gnu' else ''
    return f''';; -*- lexical-binding: t -*-
(defvar bench-init-errors nil "Init blocks that signalled, as (NAME MESSAGE).")
(defmacro bench-block (name &rest body)
  "Run BODY; record a signal under NAME and go on with the init."
  `(condition-case err (progn ,@body)
     (error (push (list ,name (error-message-string err)) bench-init-errors)
            (message "bench init: %s: %s" ,name (error-message-string err)))))
{f"(setq load-path (cons {lisp_string(str(lisp))} load-path))" if owner else ""}
{gnu_setup}(setq inhibit-startup-screen t)
{OWNER_INIT if owner else ""}
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


def populate(home, project, org, gnosis_db):
    """Copy the session's data into HOME; return what was used."""
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
        gnosis_db = Path(gnosis_db).resolve()
        if gnosis_db == LIVE_GNOSIS_DB.resolve():
            raise ValueError('refusing the live gnosis database; give a copy')
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


def lisp_dir(base, engine):
    """The owner's package sources copied once under BASE/lisp (a directory
    of its own: a layer's plan lists a package directory's siblings);
    GNU's compiled."""
    (Path(base) / 'lisp').mkdir(exist_ok=True)
    return parity_live.lisp_copy(Path(base) / 'lisp', f'owner-{engine}', sources(), engine == 'gnu')
