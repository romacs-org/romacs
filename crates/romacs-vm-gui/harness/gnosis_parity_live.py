#!/usr/bin/env python3
"""Gnosis in pinned GNU and in Romacs GUI --loadup, side by side, on a DB copy.

Each workflow starts a fresh pair of sessions: pinned GNU Emacs (pgtk) and
`romacs-vm-gui --loadup`, each in its own private headless labwc with a private
HOME/XDG/TMPDIR and its own copy of a Gnosis database.  pgtk under
labwc, not -nw over a pty: both engines then take the same Wayland key events
from wtype through the same compositor and both are graphical frames, so
`display-graphic-p' code paths, faces and images are a graphical session's.

Both inits load parity_probe.el, whose timer answers Lisp requests
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
DB and REPO default to a profile config's gnosis_db and gnosis_db_repo
(--profile-config or $ROMACS_BENCH_PROFILE, real_init.py).
DB is a copy of a gnosis.db, never the live file (refused); its images/ and
assets/ siblings are copied with it.  REPO is a Git repository whose only
commit holds that copy as gnosis.db: each session clones it (--shared), so
gnosis's automatic commits after a review have a history to land on, as in
a real gnosis-dir.  Nothing here writes DB, REPO or their directory.
"""
import os
from pathlib import Path
import re
import subprocess
import sys

from parity_live import (ROOT, C, Harness, K, M, MODE_IS, POPUP_HAS, PROMPT_IS, BUFFER_IS, T, gnu_engine, key,
                         lisp_copy, lisp_list, lisp_string, main, mx, private_tree, romacs_engine, typed)
import parity_live
import real_init

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
#     holds and no one-shot timer the step started is due within 3s
#     (parity-probe-quiet), so work a step deferred to a timer has run on
#     both sides.
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


# --- the shared configuration ------------------------------------------------

def init_source(engine, root, lisp):
    """The init both engines load: identical except where each finds gnosis."""
    # GNU gets the packages Romacs bundles, installed as package.el would:
    # their directories on `load-path' and the same generated autoloads
    # Romacs's --loadup loads, whose lisp/romacs/ libraries (romacs-video's
    # find-file-hook) need lisp/romacs/ too.  gnosis and keymap-popup come byte-compiled.
    bundled = [lisp] + [ROOT / 'lisp/romacs' / d for d in ('', 'hermes', 'jabber', 'deps/fsm', 'deps/markdown-mode', 'deps/websocket')]
    load = '' if lisp is None else (
        f'(setq load-path (append (list {" ".join(lisp_string(str(d)) for d in bundled[:1])}) load-path'
        f' (list {" ".join(lisp_string(str(d)) for d in bundled[1:])})))\n'
        f'(load {lisp_string(str(ROOT / "lisp/romacs/romacs-bundled-autoloads.el"))} nil t)\n')
    # GNU takes the settings Romacs's --loadup makes before the user's init:
    # its completion defaults (the same file), its scroll-conservatively
    # (romacs-startup.el) and no GUI chrome Romacs lacks.
    gnu_setup = ('(menu-bar-mode -1) (tool-bar-mode -1) (scroll-bar-mode -1)\n'
                 '(setq scroll-conservatively 101)\n'
                 f'(load {lisp_string(str(ROOT / "lisp/romacs/romacs-completion-defaults.el"))} nil t)\n') if engine == 'gnu' else ''
    return f''';; -*- lexical-binding: t -*-
{load}{gnu_setup}{parity_live.GNU_COMPLETION}(setq inhibit-startup-screen t initial-scratch-message nil)
(setq gnosis-dir {lisp_string(str(root / 'gnosis') + '/')})
(windmove-mode 1)
(random "gnosis-parity")
(load {lisp_string(str(parity_live.PROBE))} nil t)
(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))
'''


def database(args):
    """(DB, REPO) from the arguments or the profile config; a live DB refused."""
    config = real_init.load(args.profile_config)
    db, repo = args.db or config.get('gnosis_db'), args.db_repo or config.get('gnosis_db_repo')
    if not (db and repo):
        raise SystemExit('needs --db and --db-repo (or a profile config with gnosis_db and gnosis_db_repo)')
    return real_init.refuse_live_gnosis(db, config), Path(repo)


def gnosis_tree(engine, base, args):
    """A private tree holding a clone of the database repository."""
    db, repo = database(args)
    root, home, env = private_tree(engine, base, args.tools)
    gitenv = {'HOME': str(home), 'PATH': os.environ['PATH'], 'GIT_CONFIG_NOSYSTEM': '1'}
    (home / '.gitconfig').write_text('[user]\n\tname = parity\n\temail = parity@localhost\n'
                                     '[commit]\n\tgpgsign = false\n[init]\n\tdefaultBranch = master\n')
    subprocess.run(['git', 'clone', '-q', '--shared', str(repo), str(root / 'gnosis')],
                   env=gitenv, check=True)
    for extra in ('images', 'assets'):
        source = db.parent / extra
        if source.is_dir():
            subprocess.run(['cp', '-r', '--reflink=auto', str(source), str(root / 'gnosis' / extra)], check=True)
    return root, home, env


def gnu_lisp(base):
    """gnosis and keymap-popup byte-compiled by the pinned GNU, as a GNU
    user runs them; cached by source hash."""
    sources = sorted((ROOT / 'lisp/romacs/gnosis').glob('*.el')) + [ROOT / 'lisp/romacs/emacs-lisp/keymap-popup.el']
    return lisp_copy(base, 'gnu-gnosis', sources, True)


def launch(engine, base, args):
    root, home, env = gnosis_tree(engine, base, args)
    if engine == 'gnu':
        lisp = gnu_lisp(base)
        return gnu_engine(root, home, env, init_source(engine, root, lisp), lisp, args.timeout, args.tools)
    return romacs_engine(args.romacs, root, home, env, init_source(engine, root, None), ROOT / 'lisp/romacs/gnosis',
                         args.timeout, args.tools)


# --- workflows ----------------------------------------------------------------

DASHBOARD = lisp_string('*Gnosis Dashboard*')
DASHBOARD_RENDERED = (f'(and {BUFFER_IS(DASHBOARD)} (with-current-buffer {DASHBOARD}'
                      ' (and (null gnosis-dashboard--timer) (null gnosis-dashboard--pending-entries))))')
DASHBOARD_READY = (f'(and {DASHBOARD_RENDERED} (with-current-buffer {DASHBOARD} (save-excursion (goto-char (point-min))'
                   ' (and (search-forward "Review streak" nil t) (not (search-forward "Loading statistics" nil t))))))')


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
        # History shows in its own buffer, *Gnosis History*.
        key('history', [T('H')], MODE_IS("gnosis-dashboard-history-mode")),
        key('history-back', [T('q')], DASHBOARD_READY),
        # Line motion and scrolling over the themata list.
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
        typed('type', 'basic', [K('Return')],
              f'(and {BUFFER_IS(lisp_string("*Gnosis NEW*"))} (save-excursion (forward-line -1) (looking-at-p "\\*\\* Keimenon")))'),
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


HARNESS = Harness(WORKFLOWS, launch, f"(parity-probe-snapshot '{lisp_list(DB_QUERIES)})", 'gnosis-parity-',
                  lambda value: re.sub(r'WPM: \d+', 'WPM: <rate>', value))


def arguments(parser):
    parser.add_argument('--profile-config', default=os.environ.get(real_init.CONFIG_ENV),
                        help='gnosis_db and gnosis_db_repo defaults (real_init.py)')
    parser.add_argument('--db', help='a copy of gnosis.db; never the live one')
    parser.add_argument('--db-repo', help='a Git repository holding that copy as gnosis.db')


if __name__ == '__main__':
    sys.exit(main(HARNESS, __doc__, arguments))
