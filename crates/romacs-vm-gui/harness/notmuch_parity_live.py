#!/usr/bin/env python3
"""The real notmuch package in pinned GNU and in Romacs GUI --loadup, side by side.

Each workflow starts a fresh pair of sessions (parity_live.py), each in its
own private headless labwc with a private HOME/XDG/TMPDIR holding its own
copy of one synthetic mail store (notmuch_fixture.py: same messages, same
dates, its own notmuch database and NOTMUCH_CONFIG) and its own loopback
SMTP server (fake_smtp.py: STARTTLS and implicit TLS under a fixture CA).
Both load notmuch from the system site-lisp (GNU its .elc, Romacs its .el)
through a mail init: --snippet FILE, a user's notmuch and smtpmail settings
as a Romacs init would hold them, or else DEFAULT_INIT below; then fixture
overrides: the fixture's identities and Fcc folders, smtpmail (and a
snippet's accounts table, --accounts-variable, address to :server :port :type)
pointed at the engine's own server, its CA as the only extra trust and a
fixture authinfo.  No real mail, notmuch database, ~/.notmuch-config or
network is touched; the mail sync command is never run.

usage: notmuch_parity_live.py --romacs BIN [--gnu EMACS] --output DIR
                              [--workflow NAME]... [--snippet FILE [--accounts-variable NAME]]

Workflows: `controls' (a search whose headers carry control characters,
no-break spaces and emoji: M->, M-<, scrolling and recentering),
`read' (hello, a saved search, motion, tagging, archiving,
notmuch-show with HTML parts, notmuch-tree), `compose-send' (a new mail
with address completion, an attachment and a saved draft, sent over
STARTTLS) and `reply-send' (a reply from the identity it was addressed to;
over implicit TLS when the init's accounts send that identity so).

COMPARISON POLICY: as gnosis_parity_live.py's -- buffers, point, text,
properties, overlays, modes, mode and header lines, windows, minibuffer,
echo area and *Messages* exactly; paths, wall-clock and geometry
normalised or reported apart -- and, normalised with the reason:
  * Message-IDs and MIME boundaries message.el makes from the clock and the
    process id become <message-id> and <boundary>; a draft's or Fcc's file
    name (time, pid, host) becomes <maildir-file>.
After a workflow that sends, each engine's captured submissions must agree
with the other's: envelope, TLS use, AUTH user and the message with those
normalised.  Neither engine may log `Error running timer', `Error during
redisplay' or a process filter or sentinel error.
"""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import time

from parity_live import (C, Harness, K, M, MODE_IS, PROMPT_IS, BUFFER_IS, T, gnu_engine, key, lisp_string, main, mx,
                         private_tree, romacs_engine, typed)
import parity_live
import notmuch_fixture

HARNESS = Path(__file__).resolve().parent
ANCHOR = int(time.time()) // 60 * 60
PROBE_VARIABLES = ['notmuch-search-oldest-first', 'notmuch-archive-tags', 'notmuch-hello-sections',
                   'message-send-mail-function', 'smtpmail-stream-type', 'mm-text-html-renderer']

# The mail init without --snippet: notmuch from the system site-lisp with
# saved searches and hello sections, sending through smtpmail.
DEFAULT_INIT = r'''
(add-to-list 'load-path "/usr/share/emacs/site-lisp")
(autoload 'notmuch-hello "notmuch" nil t)
(keymap-global-set "C-x m" #'notmuch-hello)
(setq notmuch-archive-tags '("-inbox" "-unread" "+archived")
      notmuch-always-prompt-for-sender t
      notmuch-hello-sections '(notmuch-hello-insert-header notmuch-hello-insert-saved-searches
                               notmuch-hello-insert-alltags)
      notmuch-saved-searches '((:name "Week's Inbox" :query "tag:inbox date:7d..today" :key "i")
                               (:name "Inbox" :query "tag:inbox" :key "I")
                               (:name "all mail" :query "*" :key "a"))
      notmuch-search-oldest-first nil
      message-send-mail-function #'smtpmail-send-it
      smtpmail-stream-type 'starttls)
(with-eval-after-load 'notmuch
  (keymap-set notmuch-hello-mode-map "s" (lambda () (interactive) (notmuch-search (notmuch-read-query "Notmuch search: ")))))
(require 'smtpmail)
'''

FIXTURE_INIT = r'''
;; The fixture's accounts, identities and trust over the mail init's.
(let* ((dir (getenv "FAKE_SMTP_DIR"))
       (ports (with-temp-buffer (insert-file-contents (expand-file-name "ports.json" dir))
                                (json-parse-buffer :object-type 'alist))))
  (setq smtpmail-smtp-server "localhost" smtpmail-smtp-service (alist-get 'starttls ports)
        gnutls-trustfiles (list (expand-file-name "ca.pem" dir)))
  (when (and '@ACCOUNTS@ (boundp '@ACCOUNTS@))
    (set '@ACCOUNTS@
          `(("owner@example.org" :user "owner@example.org" :server "localhost"
             :port ,(alist-get 'starttls ports) :type starttls)
            ("owner@students.example.org" :user "owner@example.org" :server "localhost"
             :port ,(alist-get 'tls ports) :type tls)))))
(with-temp-file "~/.authinfo"
  (insert "machine localhost login owner@example.org password fixture-password\n"))
(setq auth-sources '("~/.authinfo")
      user-full-name "Owner Example" user-mail-address "owner@example.org"
      notmuch-identities '("Owner Example <owner@example.org>" "Owner Student <owner@students.example.org>")
      notmuch-fcc-dirs '(("owner@example\\.org" . "mail-vps/Sent +sent -new -inbox -unread")
                         (".*" . "sent +sent -new -inbox -unread"))
      message-signature "Owner Example\nhttps://example.org")
'''


def init_source(engine, snippet, accounts=None):
    mail = f'(load {lisp_string(str(snippet))} nil t)' if snippet else DEFAULT_INIT
    gnu_setup = ('(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
                 '(setq scroll-conservatively 101)\n'
                 f'(load {lisp_string(str(parity_live.ROOT / "rlisp/romacs-completion-defaults.el"))} nil t)\n'
                 ) if engine == 'gnu' else ''
    variables = ' '.join(PROBE_VARIABLES)
    return f''';; -*- lexical-binding: t -*-
{gnu_setup}{parity_live.GNU_COMPLETION}(setq inhibit-startup-screen t initial-scratch-message nil)
{mail}
{FIXTURE_INIT.replace('@ACCOUNTS@', accounts or 'nil')}
(setq parity-probe-variables '({variables}))
(random "notmuch-parity")
(load {lisp_string(str(parity_live.PROBE))} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''


def launch(engine, base, args):
    root, home, env = private_tree(engine, base, args.tools)
    env.update(notmuch_fixture.build(home, ANCHOR))
    (home / 'notes.txt').write_text('Attachment fixture: Zoë’s notes ✓\n')
    smtp = root / 'smtp'
    server = subprocess.Popen([sys.executable, '-I', str(HARNESS / 'fake_smtp.py'), str(smtp)],
                              stdout=subprocess.DEVNULL, stderr=(root / 'smtp.log').open('w'))
    parity_live.wait(lambda: (smtp / 'ports.json').exists(), 30, 'fake SMTP server', server)
    env['FAKE_SMTP_DIR'] = str(smtp)
    init = init_source(engine, args.snippet and Path(args.snippet).resolve(), args.accounts_variable)
    engine_ = (gnu_engine(root, home, env, init, None, args.timeout, args.tools) if engine == 'gnu'
               else romacs_engine(args.romacs, root, home, env, init, None, args.timeout, args.tools))
    engine_.processes.insert(0, server)
    return engine_


def rewrite(text):
    text = re.sub(r'<[0-9a-z]+\.fsf@[^>]*>', '<message-id>', text)
    text = re.sub(r'=-=-?[0-9a-zA-Z=+/_-]*=-=', '<boundary>', text)
    text = re.sub(r'\b\d{10}\.M\d+P\d+\.[^/:\s"]+(:2,[A-Z]*)?', '<maildir-file>', text)
    return text


# --- workflows -------------------------------------------------------------

def in_window(form):
    return f'(with-current-buffer (window-buffer (selected-window)) {form})'


IDLE = '(not (get-buffer-process (window-buffer (selected-window))))'
HELLO = f'(and {BUFFER_IS(lisp_string("*notmuch-hello*"))} {MODE_IS("notmuch-hello-mode")})'
SEARCH = f'(and {MODE_IS("notmuch-search-mode")} {IDLE})'
SHOW = f'(and {MODE_IS("notmuch-show-mode")} {IDLE})'
TREE = f'(and {MODE_IS("notmuch-tree-mode")} {IDLE})'
MINIBUFFER = '(minibufferp (window-buffer (selected-window)))'
COMPOSE = MODE_IS('notmuch-message-mode')
SENT = '(file-exists-p (expand-file-name {} (getenv "FAKE_SMTP_DIR")))'.format
LINE_HAS = in_window('(save-excursion (beginning-of-line) (search-forward {} (line-end-position) t))').format

READ = [
    key('hello', [C('x'), T('m')], HELLO),
    key('jump-week', [T('j'), T('i')], SEARCH, settle=1),
    key('next', [T('n')], SEARCH),
    key('next-2', [T('n')], SEARCH),
    key('previous', [T('p')], SEARCH),
    key('tag-prompt', [T('+')], MINIBUFFER),
    typed('tag', 'todo', [K('Return')], f'(and {SEARCH} {LINE_HAS(lisp_string("todo"))})'),
    key('archive', [T('a')], SEARCH),
    key('show', [K('Return')], SHOW, settle=1),
    key('show-next', [T('n')], SHOW),
    key('show-scroll', [C('v')], SHOW),
    key('show-scroll-back', [M('v')], SHOW),
    key('show-end', [M('>')], SHOW),
    key('quit-show', [T('q')], SEARCH),
    dict(name='tree-prompt', **mx('notmuch-tree'), ready=MINIBUFFER),
    typed('tree', 'tag:inbox', [K('Return')], TREE, settle=1),
    key('tree-next', [T('n')], TREE),
    key('tree-show', [K('Return')], f'(and {TREE} (get-buffer-window (seq-find (lambda (b) (eq (buffer-local-value (quote major-mode) b) (quote notmuch-show-mode))) (buffer-list))))', settle=1),
    key('tree-quit', [T('q')], TREE),
    key('tree-quit-2', [T('q')], SEARCH),
    key('quit-search', [T('q')], HELLO),
]

COMPOSE_SEND = [
    key('hello', [C('x'), T('m')], HELLO),
    key('new-mail', [T('m')], MINIBUFFER),
    key('sender', [K('Return')], f'(and {COMPOSE} {LINE_HAS(lisp_string("To:"))})', settle=1),
    dict(name='to-complete', keys=[T('Zo')], keys_ready=LINE_HAS(lisp_string('Zo')), trigger=[K('Tab')],
         ready=LINE_HAS(lisp_string('zoe@example.org')), settle=1),
    key('goto-subject', [C('c'), C('f'), C('s')], f'(and {COMPOSE} {LINE_HAS(lisp_string("Subject:"))})'),
    dict(name='subject', keys=[T('Parity ünïcode test')], keys_ready=LINE_HAS(lisp_string('ünïcode')), trigger=[C('c'), C('b')],
         ready=f'(and {COMPOSE} (eolp))'),
    dict(name='body', keys=[T('Hello Zoë,'), K('Return'), K('Return'), T('A parity test — ✓.'), K('Return')],
         keys_ready=in_window('(save-excursion (search-backward "parity test" nil t))'), trigger=[C('c'), C('a')], ready=MINIBUFFER),
    typed('attach-file', '~/notes.txt', [K('Return')], MINIBUFFER),
    key('attach-type', [K('Return')], MINIBUFFER),
    key('attach-description', [K('Return')], MINIBUFFER),
    key('attach-disposition', [K('Return')], f'(and {COMPOSE} {in_window("(save-excursion (goto-char (point-min)) (search-forward \"<#part\" nil t))")})'),
    key('save-draft', [C('x'), C('s')], f'(and {COMPOSE} {in_window("(not (buffer-modified-p))")})', settle=1),
    key('send', [C('c'), C('c')], f'(and (not {COMPOSE}) {SENT(lisp_string("001.eml"))})', settle=2),
]

REPLY_SEND = [
    key('hello', [C('x'), T('m')], HELLO),
    key('search-prompt', [T('s')], MINIBUFFER),
    typed('search', 'to:owner@students.example.org', [K('Return')], SEARCH, settle=1),
    key('show', [K('Return')], SHOW, settle=1),
    key('reply', [T('r')], f'(and {COMPOSE} {IDLE})', settle=1),
    dict(name='reply-body', keys=[T('Replying over implicit TLS.')], keys_ready=LINE_HAS(lisp_string('implicit TLS')),
         trigger=[C('c'), C('c')], ready=f'(and (not {COMPOSE}) {SENT(lisp_string("001.eml"))})', settle=2),
]

# Headers with control characters, no-break spaces and emoji: motion to
# the ends and scrolling keep point and the window start as GNU's.
CONTROLS = [
    key('hello', [C('x'), T('m')], HELLO),
    key('search-prompt', [T('s')], MINIBUFFER),
    typed('search', 'tag:controls', [K('Return')], SEARCH, settle=1),
    key('end', [M('>')], SEARCH),
    key('beginning', [M('<')], SEARCH),
    key('end-again', [M('>')], SEARCH),
    key('scroll-back', [M('v')], SEARCH),
    key('scroll', [C('v')], SEARCH),
    key('beginning-again', [M('<')], SEARCH),
    key('next', [T('n')], SEARCH),
    key('recenter', [C('l')], SEARCH),
    key('quit-search', [T('q')], HELLO),
]

WORKFLOWS = {'read': READ, 'compose-send': COMPOSE_SEND, 'reply-send': REPLY_SEND, 'controls': CONTROLS}


def normalised_mail(engine):
    """The engine's captured submissions, with rewrite and its paths normalised."""
    smtp = engine.root / 'smtp'
    out = []
    for envelope in sorted(smtp.glob('[0-9][0-9][0-9].json')):
        record = json.loads(envelope.read_text())
        record['from'] = re.sub(r' SIZE=\d+', ' SIZE=<n>', record['from'])
        message = envelope.with_suffix('.eml').read_bytes().decode('utf-8', 'replace')
        message = re.sub(r'^Date: .*$', 'Date: <date>', message, flags=re.M)
        record['message'] = rewrite(message.replace(str(engine.root), '<root>'))
        out.append(record)
    return out


def check(engines):
    marks = ('Error running timer', 'Error during redisplay', 'error in process filter', 'error in process sentinel')
    out = {'errors': []}
    for engine in engines:
        messages = engine.ask('(with-current-buffer (messages-buffer) (buffer-substring-no-properties (min parity-probe-messages-start (point-max)) (point-max)))')
        log = (engine.root / 'engine.log').read_text(errors='replace') if (engine.root / 'engine.log').exists() else ''
        found = [line for line in (messages.splitlines() + log.splitlines()) if any(m in line for m in marks)]
        out[engine.name] = found
        out['errors'] += [f'{engine.name}: {line}' for line in found]
    mail = {engine.name: normalised_mail(engine) for engine in engines}
    out['mail'] = mail
    if mail.get('gnu') != mail.get('romacs'):
        out['errors'].append('captured submissions differ')
    return out


def add_arguments(parser):
    parser.add_argument('--snippet', help="a mail init fragment, instead of DEFAULT_INIT")
    parser.add_argument('--accounts-variable', help="the snippet's accounts table, pointed at the fixture's servers")


HARNESS_SPEC = Harness(WORKFLOWS, launch, seed='notmuch-parity-', rewrite=rewrite, check=check)

if __name__ == '__main__':
    sys.exit(main(HARNESS_SPEC, __doc__, add_arguments))
