#!/usr/bin/env python3
"""Hermes in pinned GNU and in Romacs GUI --loadup, side by side, read-only.

Each workflow starts a fresh pair of sessions (parity_live.py): pinned GNU
Emacs (pgtk) and `romacs-vm-gui --loadup', each in its own private headless
labwc with a private HOME/XDG/TMPDIR.  Both load the owner's hermes checkout
and the dependencies the owner's straight recipes load (keymap-popup,
websocket, markdown-mode), copied once into the work directory: GNU's copy
byte-compiled, as straight builds it, Romacs's as source.  Nothing here
writes the checkouts or ~/.config.

The backend is the owner's running dashboard at 127.0.0.1:9119, attached
(`hermes-dashboard-transport-start-mode' remote), never started.  No
credential is supplied: hermes's loopback handshake discovers the
dashboard's session token, which stays in the engine's memory; the probe
reports "<secret>" for any string holding it, and nothing writes it.  The
workflows only read: the dashboard, the session browser, a session's
history, its search and quitting back.  They never press a key that
renames, pins, archives, deletes, exports or resumes a session, or that
sends anything -- but for the one write the owner authorized: the `chat'
workflow opens a new chat, whose session the backend creates titled
`romacs-parity-test' (hermes-chat-dashboard-session-title, set at creation,
never a rename), sends `hello' and waits for the reply, once in each
engine.  `chat-dry' runs its steps up to that send, writing nothing.

usage: hermes_parity_live.py --romacs BIN [--gnu EMACS] --output DIR
                             [--workflow NAME]... [--perf]

Without --workflow it runs the read-only ones; `chat' runs only by name.

COMPARISON POLICY: as gnosis_parity_live.py's -- buffers, point, text,
properties, overlays, modes, mode and header lines, windows, minibuffer,
echo area and *Messages* exactly; paths, wall-clock and geometry
normalised or reported apart -- with the package state the probe records
being `PROBE_VARIABLES' below rather than every `hermes' variable, since
hermes's own variables hold the token, and with neither engine asking
before killing a buffer with a live process (see init_source).  The backend's data is live: the
engines run each step seconds apart, so a session that gains a message in
between shows up as a difference in the list; the history the workflow
opens is an old, closed session's.  In `chat' the reply's words are the
model's and differ between the engines; its rows, faces, status rule and
composer are compared as they are, and its arrival is the send step's
time.  After every workflow neither engine may have logged `Error running
timer' or `Error during redisplay' in *Messages* or on stderr.
"""
from pathlib import Path
import sys

from parity_live import (ROOT, C, Harness, M, MODE_IS, POPUP_HAS, PROMPT_IS, BUFFER_IS, T, gnu_engine, key,
                         lisp_copy, lisp_string, main, mx, private_tree, romacs_engine)
import parity_live

DASHBOARD_URL = 'http://127.0.0.1:9119'
TEST_TITLE = 'romacs-parity-test'
# The owner's checkouts, as ~/.config/emacs/init.el's straight recipes name them.
SOURCES = [
    (Path.home() / 'Projects/emacs-lisp/emacs-hermes/lisp', '*.el'),
    (Path.home() / 'Projects/emacs-lisp/keymap-popup', 'keymap-popup.el'),
    (Path.home() / '.config/emacs/straight/repos/emacs-websocket', 'websocket.el'),
    (Path.home() / '.config/emacs/straight/repos/markdown-mode', 'markdown-mode.el'),
]
# Hermes's browser and history state; none of these holds a credential.
PROBE_VARIABLES = [
    'hermes-browser--status', 'hermes-sessions--search-query', 'hermes-sessions--search-profile',
    'hermes-sessions--archived-filter', 'hermes-sessions--catalogue-offset', 'hermes-sessions--all-profiles',
    'hermes-sessions--detail-count', 'tabulated-list-sort-key', 'tabulated-list-format',
]


def sources():
    return sorted(p for directory, pattern in SOURCES for p in directory.glob(pattern))


def init_source(engine, lisp):
    """The init both engines load: the same but for GNU's chrome and Romacs's
    defaults, which GNU takes as gnosis_parity_live.py's GNU does."""
    gnu_setup = ('(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
                 '(setq scroll-conservatively 101)\n'
                 f'(load {lisp_string(str(ROOT / "rlisp/romacs-completion-defaults.el"))} nil t)\n') if engine == 'gnu' else ''
    variables = ' '.join(PROBE_VARIABLES)
    return f''';; -*- lexical-binding: t -*-
(setq load-path (cons {lisp_string(str(lisp))} load-path))
{gnu_setup}{parity_live.GNU_COMPLETION}(setq inhibit-startup-screen t initial-scratch-message nil)
;; The owner's hermes settings, with every instance the local dashboard.
(setq hermes-instances '(("local" . {lisp_string(DASHBOARD_URL)}))
      hermes-dashboard-transport-url {lisp_string(DASHBOARD_URL)}
      hermes-dashboard-transport-start-mode 'remote
      ;; A session this harness creates is titled as a test's.
      hermes-chat-dashboard-session-title {lisp_string(TEST_TITLE)})
(require 'hermes)
;; The owner's init.el numbers every buffer's lines, relative to point's.
(setq display-line-numbers-type 'relative)
(global-display-line-numbers-mode 1)
;; The owner's hermes (e8cf489) kills an http buffer that another live
;; connection still holds, so `kill-buffer' would ask about that process at
;; a moment request timing chooses, in GNU and Romacs alike.  Neither asks.
(setq kill-buffer-query-functions (delq 'process-kill-buffer-query-function kill-buffer-query-functions))
(defun hermes-parity--strings (value)
  (cond ((stringp value) (list value))
        ((consp value) (append (hermes-parity--strings (car value)) (hermes-parity--strings (cdr value))))))
(defun hermes-parity-secrets ()
  "The credentials hermes holds for its dashboard."
  (let ((auth (bound-and-true-p hermes-dashboard-transport--api-auth)))
    (append (hermes-parity--strings (plist-get auth :discovered-token))
            (hermes-parity--strings (plist-get auth :secrets))
            (hermes-parity--strings (bound-and-true-p hermes-dashboard-transport--native-token-memory)))))
(setq parity-probe-variables '({variables})
      parity-probe-secrets-function #'hermes-parity-secrets)
(random "hermes-parity")
(load {lisp_string(str(parity_live.PROBE))} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''


def launch(engine, base, args):
    root, home, env = private_tree(engine, base, args.tools)
    lisp = lisp_copy(base, f'hermes-{engine}', sources(), engine == 'gnu')
    if engine == 'gnu':
        return gnu_engine(root, home, env, init_source(engine, lisp), lisp, args.timeout, args.tools)
    return romacs_engine(args.romacs, root, home, env, init_source(engine, lisp), lisp, args.timeout, args.tools)


# --- workflows ----------------------------------------------------------------

DASHBOARD = lisp_string('*Hermes Dashboard*')
SESSIONS = lisp_string('*Hermes Sessions*')
SESSIONS_READY = (f'(and {BUFFER_IS(SESSIONS)} {MODE_IS("hermes-sessions-mode")}'
                  f' (equal (buffer-local-value (quote hermes-browser--status) (get-buffer {SESSIONS})) "Ready"))')
DETAIL_READY = (f'(and {MODE_IS("hermes-session-detail-mode")}'
                ' (save-excursion (goto-char (point-min)) (search-forward "Messages: " nil t)))')
ISEARCHING = '(and isearch-mode (equal isearch-string {}))'.format

CHAT = '(eq (buffer-local-value (quote major-mode) (window-buffer (selected-window))) (quote hermes-chat-mode))'
IN_CHAT = '(with-current-buffer (window-buffer (selected-window)) {})'.format
# A turn has ended once the rule recorded its duration and none runs.
REPLIED = IN_CHAT('(and hermes-chat--status-rule-last (not hermes-chat--status-rule-start) (not (hermes-chat--active-turn-p)))')
# Up to the send: open a new chat from the dashboard with the default
# profile and type the message.  Nothing here reaches the backend's sessions.
CHAT_STEPS = [
    dict(name='open', **mx('hermes'), ready=f'(and {BUFFER_IS(DASHBOARD)} {POPUP_HAS(lisp_string("Sessions"))})'),
    key('new-chat', [T('c')], '(and (minibufferp (window-buffer (selected-window))) (string-prefix-p "Profile" (minibuffer-prompt)))'),
    key('default-profile', [['-k', 'Return']], f'(and {CHAT} {IN_CHAT("(hermes-chat--input-position)")})', settle=3),
    dict(name='type', keys=[T('hello')], keys_ready=IN_CHAT('(equal (buffer-substring-no-properties (hermes-chat--input-position) (point-max)) "hello")'),
         trigger=[], ready='t'),
]

# Request steps settle 3s, for the requests a step starts in the background.
WORKFLOWS = {
    # The dashboard, the session browser and its motion, the newest
    # session's history read with motion and isearch, back to the browser,
    # a server-side search, and back to the dashboard.
    'sessions': [
        dict(name='open', **mx('hermes'), ready=f'(and {BUFFER_IS(DASHBOARD)} {POPUP_HAS(lisp_string("Sessions"))})'),
        key('sessions', [T('S')], SESSIONS_READY, settle=3),
        key('next-line', [T('n')]),
        key('next-line-2', [T('n')]),
        key('previous-line', [T('p')]),
        key('scroll-up', [C('v')]),
        key('end-of-buffer', [M('>')]),
        key('beginning-of-buffer', [M('<')]),
        key('view', [T('v')], DETAIL_READY, settle=3),
        key('detail-scroll-up', [C('v')]),
        key('detail-scroll-up-2', [C('v')]),
        key('detail-scroll-down', [M('v')]),
        key('detail-end', [M('>')]),
        key('detail-beginning', [M('<')]),
        key('isearch', [C('s'), T('steam')], ISEARCHING('"steam"')),
        key('isearch-next', [C('s')], ISEARCHING('"steam"')),
        key('isearch-exit', [['-k', 'Return']], '(not isearch-mode)'),
        key('quit-detail', [T('q')], SESSIONS_READY),
        key('search-prompt', [T('s')], PROMPT_IS(lisp_string('Search sessions: '))),
        dict(name='search', keys=[T('nginx')], keys_ready='(equal (minibuffer-contents-no-properties) "nginx")',
             trigger=[['-k', 'Return']], ready=PROMPT_IS(lisp_string('Profile (blank for dashboard default): '))),
        key('search-profile', [['-k', 'Return']],
            f'(and {SESSIONS_READY} (equal (buffer-local-value (quote hermes-sessions--search-query) (get-buffer {SESSIONS})) "nginx"))',
            settle=3),
        key('quit-sessions', [T('q')], BUFFER_IS(DASHBOARD)),
    ],
    'chat-dry': CHAT_STEPS,
    # The one write: RET sends `hello'; the step lasts until the reply ends.
    'chat': CHAT_STEPS + [key('send', [['-k', 'Return']], REPLIED, settle=3)],
}
WRITES = {'chat'}


def check(engines):
    """Errors either engine logged while the workflow ran: *Messages* lines
    and stderr lines naming a timer or redisplay error."""
    marks = ('Error running timer', 'Error during redisplay')
    out = {'errors': []}
    for engine in engines:
        messages = engine.ask('(with-current-buffer (messages-buffer) (buffer-substring-no-properties (min parity-probe-messages-start (point-max)) (point-max)))')
        log = (engine.root / 'engine.log').read_text(errors='replace') if (engine.root / 'engine.log').exists() else ''
        found = [line for line in (messages.splitlines() + log.splitlines()) if any(m in line for m in marks)]
        out[engine.name] = found
        out['errors'] += [f'{engine.name}: {line}' for line in found]
    return out

HARNESS = Harness(WORKFLOWS, launch, seed='hermes-parity-', check=check, writes=WRITES)


if __name__ == '__main__':
    sys.exit(main(HARNESS, __doc__))
