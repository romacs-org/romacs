#!/usr/bin/env python3
"""Typing while jabber catches up its archives: key latency against jabber's work.

One isolated `romacs-vm-gui --loadup' session (private HOME/XDG/TMPDIR,
headless labwc, keys typed with wtype into that compositor only) shows a
notmuch search over the synthetic store (notmuch_fixture.py) above a text
file.  Jabber connects (`jabber-connect-all') to a private XMPP server
(xmpp_fixture.py: rooms with 5,000-message archives and occupant floods,
a personal archive, live traffic) or, with --account real, to an account
a user's init configures.  While the archives catch up, the harness types
bursts of C-n/C-p in the notmuch window and of letters in the text window
(F6 and F5 select them) for --duration seconds, then waits until the
catch-up has finished: every expected archive settled, no job running or
pending, no buffer left to merge.

A key's latency is from just before wtype typed it to the end of its
command (`post-command-hook', by the wall clock).  The session records,
besides every command, the spans of jabber's work on the main VM: its
state machine's events (process filters and stanza handlers run there),
the timers of jabber's functions, the isolate deliveries, the background
sync's own steps (spawn, completion, merge) and its SQLite statements.  A
key over --slow (16 ms) is *associated* with jabber when such a span
overlaps it: overlap shows association, not cause, and the noise floor
(variant `none') has slow keys with no jabber in them.  It is
*attributable* to that work when the work's time within the key covers
the key's excess over --slow: without it the key would have been fast.  Keys are reported
for the whole typing and for the catch-up's window alone.

Variants: `session' (jabber's own in-session catch-up, --page results a
page), `worker' (the catch-up in a worker VM, `jabber-mam-background'),
`none' (jabber never connects: the noise floor).

--restricted (implied by --account real) makes the live connection
read-only and quiet: no presence, no room joined, no Stream Management,
carbons, client state or avatar publication, and a send gate (the
background sync's, plus roster, bookmark and disco#items reads) that
refuses anything else by signalling.  The rooms to catch up come from
--rooms (fixture) or the bookmarks (read).  The run fails if the fixture
saw a presence or message from the client, or the gate refused anything
(with --push-bookmark, anything but the join's presence).

A run fails (exit 1, `problems') when the catch-up did not finish, an
archive failed, the worker variant ran no worker, the typed keys and the
recorded commands do not pair, or (worker variant) a key typed during the
catch-up was over --slow because of the background sync's work.

usage: jabber_live.py --binary PATH --output DIR [--variant session|worker|none]
                      [--page 50] [--duration 60] [--account fixture|real]
                      [--restricted] [--push-bookmark] [--backlog 30]
                      [--tools DIR] [--seed 1]
"""
import argparse
import json
import os
from pathlib import Path
import random
import shutil
import socket
import subprocess
import sys
import time

import parity_live
from parity_live import lisp_string, private_tree, romacs_engine
import notmuch_fixture

HARNESS = Path(__file__).resolve().parent
ANCHOR = int(time.time()) // 60 * 60
ROOMS = 'busy1:5000:800,busy2:5000:400,quiet1:200:20'

INIT = r'''
(add-to-list 'load-path "/usr/share/emacs/site-lisp")
(autoload 'notmuch-search "notmuch" nil t)
(setq auto-save-timeout nil auto-save-interval 0
      ;; A warning's buffer would take a third window (the isolated session
      ;; has no system-sleep for jabber's keepalive): log it, show none.
      warning-minimum-level :emergency)
(defvar journey-records nil "Per command: (COMMAND EVENT GCS GC-ELAPSED TIME).")
(defvar journey-spans nil "Jabber's main-VM work: (START END WHAT), newest first.")
(defvar journey-refresh-phases nil "Refresh steps: (START END STEP), newest first.")
(add-hook 'post-command-hook
          (lambda () (push (list this-command last-command-event gcs-done gc-elapsed (float-time)) journey-records)))
(defun journey--span (what function args)
  (let ((start (float-time)))
    (unwind-protect (apply function args)
      (push (list start (float-time) what) journey-spans))))
(defvar journey--depth 0)
(defun journey--wrap (what)
  (lambda (function &rest args)
    (if (> journey--depth 0)
        (apply function args)
      (let ((journey--depth 1)) (journey--span what function args)))))
(defun journey--jabber-function-p (function)
  (and (symbolp function) (string-match-p "\\`\\(jabber\\|fsm\\)-" (symbol-name function))))
(advice-add 'timer-event-handler :around
            (lambda (function timer)
              (if (and (= journey--depth 0) (journey--jabber-function-p (timer--function timer)))
                  (let ((journey--depth 1)) (journey--span (timer--function timer) function (list timer)))
                (funcall function timer))))
(with-eval-after-load 'fsm (advice-add 'fsm-send-sync :around (journey--wrap 'fsm)))
(with-eval-after-load 'isolate
  (advice-add 'romacs--isolate-dispatch :around (journey--wrap 'isolate)))
(defvar journey-sqlite nil "Main-VM statements on jabber's database: (START END SQL), newest first.")
(advice-add 'sqlite-execute :around
            (lambda (function db &rest args)
              (let ((start (float-time)))
                (unwind-protect (apply function db args)
                  (when (and (boundp 'jabber-db--connection) (eq db jabber-db--connection))
                    (push (list start (float-time) (substring (car args) 0 (min 30 (length (car args)))))
                          journey-sqlite))))))
(defvar journey-notmuch-window nil)
(defvar journey-text-window nil)
(defun journey-select-notmuch () "Select the journey's notmuch window." (interactive)
  (select-window journey-notmuch-window))
(defun journey-select-text () "Select the journey's text window." (interactive)
  (select-window journey-text-window))
(global-set-key [f5] #'journey-select-notmuch)
(global-set-key [f6] #'journey-select-text)
(defun journey-windows (text)
  (notmuch-search "tag:inbox")
  (delete-other-windows)
  (setq journey-notmuch-window (selected-window)
        journey-text-window (split-window-below))
  (with-selected-window journey-text-window (find-file text))
  ;; Nothing jabber shows takes these windows' place.
  (set-window-dedicated-p journey-notmuch-window t)
  (set-window-dedicated-p journey-text-window t)
  (redisplay t)
  (list (buffer-name) major-mode))
(defvar journey-profile-callees '(vertical-motion line-pixel-height posn-at-point
                                   jabber-muc--nick-color jabber-chat-pp--muc-notice)
  "Functions whose callers (up to three frames) the profile dump lists.")
(defun journey-profile-dump (file)
  "Write the CPU profile since `profiler-start' to FILE: self, cumulative,
and the callers of `journey-profile-callees'."
  (let ((log (profiler-cpu-log)) (self (make-hash-table :test #'eq))
        (cum (make-hash-table :test #'eq)) (callers (make-hash-table :test #'equal)) (total 0))
    (profiler-stop)
    (maphash (lambda (backtrace count)
               (setq total (+ total count))
               (let ((frames (delq nil (append backtrace nil))) (seen nil))
                 (when frames (puthash (car frames) (+ count (gethash (car frames) self 0)) self))
                 (dolist (f frames)
                   (unless (memq f seen) (push f seen) (puthash f (+ count (gethash f cum 0)) cum)))
                 (let ((tail (seq-some (lambda (f) (memq f frames)) journey-profile-callees)))
                   (when tail
                     (let ((key (seq-take tail 4)))
                       (puthash key (+ count (gethash key callers 0)) callers))))))
             (if (hash-table-p log) log (car log)))
    (with-temp-file file
      (insert (format ";; total %d\n;; self\n" total))
      (dolist (table (list self cum callers))
        (let (rows)
          (maphash (lambda (k v) (push (cons v k) rows)) table)
          (dolist (row (seq-take (sort rows (lambda (a b) (> (car a) (car b)))) 80))
            (insert (format "%7d %S\n" (car row) (cdr row)))))
        (insert (if (eq table cum) ";; callers\n" ";; cumulative\n"))))))
(defun journey-window-buffers ()
  (mapcar (lambda (w) (buffer-name (window-buffer w))) (window-list nil 'never)))
(defun journey--dump (file rows)
  (with-temp-file file
    (dolist (row rows) (prin1 row (current-buffer)) (terpri (current-buffer)))))
(defun journey-dump (directory)
  (journey--dump (expand-file-name "records.eld" directory)
                 (mapcar (lambda (r) (list (car r) (nth 1 r) (nth 2 r) (nth 3 r) (nth 4 r))) (reverse journey-records)))
  (journey--dump (expand-file-name "spans.eld" directory) (reverse journey-spans))
  (journey--dump (expand-file-name "sqlite.eld" directory) (reverse journey-sqlite))
  (journey--dump (expand-file-name "refresh-phases.eld" directory) (reverse journey-refresh-phases))
  (when (boundp 'journey-events)
    (journey--dump (expand-file-name "events.eld" directory) (reverse journey-events)))
  (when (boundp 'journey-passes)
    (journey--dump (expand-file-name "passes.eld" directory) (reverse journey-passes)))
  (when (boundp 'journey-slow-timers)
    (journey--dump (expand-file-name "slow-timers.eld" directory) (reverse journey-slow-timers)))
  (when (fboundp 'journey-db-identities)
    (journey--dump (expand-file-name "identities.eld" directory) (journey-db-identities)))
  (length journey-records))
'''

JABBER = r'''
(require 'jabber)
(defvar journey-mam-done nil "When the expected catch-up was last seen finished.")
(defvar journey-catch-up-start nil "When the first catch-up began: its first page, or its job.")
(defvar journey-outcomes nil "In-session queries settled: (ARCHIVE . OUTCOME), newest first.")
(defvar journey-sync-summaries nil "Background syncs' (ACCOUNT SUMMARY TIME), newest first.")
(defvar journey-posts nil "The running sync's reports: (TIME HISTORIES), newest first.")
(with-eval-after-load 'jabber-mam-sync
  (advice-add 'jabber-mam-sync--receive :before
              (lambda (_sender message)
                (when (eq (car-safe message) 'jabber-mam-sync)
                  (push (list (float-time) (length (nth 3 message))) journey-posts)))))
(defvar journey-events nil "FSM events over 5 ms: (START SECONDS WHAT), newest first.")
(defun journey--event-summary (event)
  "A short description of FSM EVENT: a stanza's name, type, sender and payload."
  (let ((kind (or (car-safe event) event)))
    (if (and (eq kind :stanza) (consp (cadr event)))
        (let* ((xml (cadr event))
               (child (car (jabber-xml-node-children xml)))
               (codes (mapcar (lambda (s) (jabber-xml-get-attribute s 'code))
                              (jabber-xml-get-children
                               (car (jabber-xml-get-children xml 'x)) 'status))))
          (list (jabber-xml-node-name xml) (jabber-xml-get-attribute xml 'type)
                (jabber-xml-get-attribute xml 'from)
                (and (consp child) (or (jabber-xml-get-xmlns child) (jabber-xml-node-name child)))
                codes))
      (list kind))))
(with-eval-after-load 'fsm
  (advice-add 'fsm-send-sync :around
              (lambda (function fsm event &rest args)
                (let ((start (float-time)))
                  (unwind-protect (apply function fsm event args)
                    (let ((took (- (float-time) start)))
                      (when (> took 0.005)
                        (push (list start took (ignore-errors (journey--event-summary event)))
                              journey-events))))))
              '((depth . -90))))
(defvar journey-passes nil "Timer passes over 10 ms: (START SECONDS TIMERS INPUT-PENDING), newest first.")
(advice-add 'romacs--timer-check :around
            (lambda (function &rest args)
              (let ((start (float-time)) (run romacs--timers-run))
                (prog1 (apply function args)
                  (let ((took (- (float-time) start)))
                    (when (> took 0.01)
                      (push (list start took (- romacs--timers-run run) (input-pending-p))
                            journey-passes)))))))
(defvar journey-slow-timers nil "Timers over 20 ms: (START SECONDS FUNCTION ARGS), newest first.")
(advice-add 'timer-event-handler :around
            (lambda (function timer)
              (let ((start (float-time)))
                (prog1 (funcall function timer)
                  (let ((took (- (float-time) start)))
                    (when (> took 0.02)
                      (push (list start took (format "%.80S" (timer--function timer))
                                  (format "%.120S" (timer--args timer)))
                            journey-slow-timers)))))))
(defvar journey-chat-window nil "The window F7 shows chats in.")
(defvar journey-chat-index 0)
(defvar journey-sent 0 "Messages F8 sent.")
(defun journey-chat-buffers ()
  (sort (seq-filter (lambda (b) (with-current-buffer b (derived-mode-p 'jabber-chat-mode))) (buffer-list))
        (lambda (a b) (string< (buffer-name a) (buffer-name b)))))
(defun journey-open-chat () "Show the next chat or room buffer." (interactive)
  (when-let* ((buffers (journey-chat-buffers)))
    (unless (window-live-p journey-chat-window)
      (setq journey-chat-window (split-window journey-text-window nil 'right)))
    (set-window-buffer journey-chat-window (nth (mod journey-chat-index (length buffers)) buffers))
    (setq journey-chat-index (1+ journey-chat-index))))
(defun journey-send () "Send a message from the shown chat." (interactive)
  (when (window-live-p journey-chat-window)
    (with-current-buffer (window-buffer journey-chat-window)
      (goto-char (point-max))
      (insert (format "journey ping %d" (setq journey-sent (1+ journey-sent))))
      (jabber-chat-buffer-send))))
(global-set-key [f7] #'journey-open-chat)
(global-set-key [f8] #'journey-send)
(defun journey-duplicates ()
  "Stored and shown duplicates: (STORED-DUPLICATES LIVE-ROWS LIVE-DISTINCT SHOWN-DUPLICATES)."
  (let ((db (jabber-db-ensure-open)) (shown 0))
    (dolist (buffer (journey-chat-buffers))
      (with-current-buffer buffer
        (let ((seen (make-hash-table :test #'equal)))
          (dolist (data (ewoc-collect jabber-chat-ewoc #'always))
            (when-let* ((id (and (listp (cadr data)) (plist-get (cadr data) :server-id))))
              (if (gethash id seen) (setq shown (1+ shown)) (puthash id t seen)))))))
    (append (car (sqlite-select db "SELECT (SELECT count(*) FROM (SELECT 1 FROM message WHERE server_id IS NOT NULL GROUP BY account, peer, server_id HAVING count(*) > 1)), count(*), count(DISTINCT server_id) FROM message WHERE server_id LIKE 'live-%'"))
            (list shown))))
(defvar journey-toasts nil "The sync's toasts: (TIME TEXT KIND), newest first.")
(advice-add 'jabber-mam--send-page :before
            (lambda (&rest _) (unless journey-catch-up-start (setq journey-catch-up-start (float-time)))))
(advice-add 'jabber-mam--complete-query :before
            (lambda (query outcome)
              (when (memq query jabber-mam--syncing)
                (push (cons (or (plist-get query :to) "") outcome) journey-outcomes))))
(defvar journey-refreshes nil "Chat buffer refreshes: (START END BUFFER ENTRIES), newest first.")
(defvar journey-refresh-errors nil "Errors a refresh's steps signalled, synchronous or in timers, newest first.")
(defvar journey-refreshing (make-hash-table :test #'eq) "Buffers whose refresh has not finished.")
(advice-add 'jabber-chat-buffer-refresh :around
            (lambda (function &rest args)
              (let ((start (float-time))
                    (before (condition-case nil (length (ewoc-collect jabber-chat-ewoc #'always)) (error -1))))
                (puthash (current-buffer) t journey-refreshing)
                (unwind-protect (apply function args)
                  (push (list start (float-time) (buffer-name) before) journey-refreshes)))))
;; A refresh ends in `jabber-chat-buffer--restore-view': directly when the
;; reload finds nothing, through the completion otherwise.
(advice-add 'jabber-chat-buffer--restore-view :after
            (lambda (&rest _) (remhash (current-buffer) journey-refreshing)))
(dolist (step '(jabber-chat-buffer--clear-chunked jabber-chat--insert-backlog-chunked
                jabber-chat-buffer--reload jabber-chat-buffer--refresh-complete jabber-mam-sync--refresh
                jabber-mam-sync--shown))
  (advice-add step :around
              (let ((step step))
                (lambda (function &rest args)
                  (let ((start (float-time)))
                    (condition-case err
                        (prog1 (apply function args)
                          (push (list start (float-time) step) journey-refresh-phases))
                      (error (push (list step (prin1-to-string err)) journey-refresh-errors)
                             (signal (car err) (cdr err)))))))))
(with-eval-after-load 'jabber-mam-sync
  (advice-add 'jabber-mam-sync--start :before
              (lambda (&rest _) (unless journey-catch-up-start (setq journey-catch-up-start (float-time)))))
  (dolist (f '(jabber-mam-sync--start jabber-mam-sync--finished jabber-mam-sync--merge-next))
    (advice-add f :around (journey--wrap f)))
  (add-hook 'jabber-mam-sync-finished-functions
            (lambda (account summary) (push (list account summary (float-time)) journey-sync-summaries)))
  (advice-add 'jabber-mam-sync--toast :before (lambda (text &optional kind) (push (list (float-time) text kind) journey-toasts))))
(setq jabber-db-path (expand-file-name "jabber.db" (getenv "HOME"))
      jabber-auto-reconnect nil
      jabber-backlog-number (string-to-number (getenv "JOURNEY_BACKLOG"))
      jabber-mam-page-size (string-to-number (getenv "JOURNEY_PAGE"))
      jabber-mam-background (equal (getenv "JOURNEY_VARIANT") "worker"))
(defun journey-open-rooms ()
  "Open each fixture room's buffer, as a user who visits the rooms has them;
none is shown in a window."
  (let ((jc (car jabber-connections)))
    (dolist (room (split-string (or (getenv "JOURNEY_ROOMS") "") "," t))
      (jabber-muc-create-buffer jc room))
    t))
(defun journey-resync ()
  "Request another catch-up of the personal archive and the fixture rooms."
  (let ((jc (car jabber-connections)))
    (jabber-mam-sync-request jc)
    (dolist (room (split-string (or (getenv "JOURNEY_ROOMS") "") "," t))
      (jabber-mam-sync-request jc room))
    t))
(defun journey-stale-buffers ()
  "Chat buffers whose merged history waits to be shown."
  (let (names)
    (dolist (buffer (buffer-list) names)
      (when (jabber-mam-sync--stale-p buffer)
        (push (buffer-name buffer) names)))))
(defun journey-show-room ()
  "Show the first fixture room in the text window: its stale history loads."
  (let* ((room (car (split-string (or (getenv "JOURNEY_ROOMS") "") "," t)))
         (buffer (jabber-muc-find-buffer room (car jabber-connections))))
    (set-window-dedicated-p journey-text-window nil)
    (set-window-buffer journey-text-window buffer)
    (redisplay t)
    (list (buffer-name buffer) (float-time))))
(defun journey-room-nodes (name)
  (with-current-buffer name (length (ewoc-collect jabber-chat-ewoc #'always))))
(defun journey-db-identities ()
  "Every stored message as (PEER TYPE ID), sorted: what variants compare."
  (ignore-errors
    (sqlite-select (jabber-db-ensure-open)
                   "select peer, type, coalesce(server_id, stanza_id, '') from message order by 1, 2, 3")))
(defun journey-expected-done-p (expected)
  "Non-nil when every EXPECTED archive (nil the personal one) settled and nothing runs or waits."
  (if (and (boundp 'jabber-mam-background) jabber-mam-background)
      (let ((archives (apply #'append
                             (mapcar (lambda (s) (mapcar #'car (plist-get (nth 1 s) :archives)))
                                     journey-sync-summaries))))
        (and journey-sync-summaries
             (cl-every (lambda (a) (member a archives)) expected)
             (catch 'busy
               (maphash (lambda (_ state)
                          (when (or (plist-get state :future) (plist-get state :pending)
                                    (plist-get state :cancelling) (plist-get state :timer))
                            (throw 'busy nil)))
                        jabber-mam-sync--accounts)
               t)
             (null jabber-mam-sync--merge-queue)
             (zerop (hash-table-count journey-refreshing))))
    (and (null jabber-mam--syncing)
         (null jabber-mam--redraw-timer)
         (zerop (hash-table-count journey-refreshing))
         (cl-every (lambda (a) (assoc (or a "") journey-outcomes)) expected))))
(defun journey-jabber-state (expected)
  (when (and (not journey-mam-done) (journey-expected-done-p expected))
    (setq journey-mam-done (float-time)))
  (list :connections (mapcar (lambda (jc) (get jc :state)) jabber-connections)
        :done journey-mam-done :start journey-catch-up-start
        :messages (caar (ignore-errors (sqlite-select (jabber-db-ensure-open) "select count(*) from message")))
        :outcomes journey-outcomes :toasts journey-toasts :summaries journey-sync-summaries
        :refreshes journey-refreshes :refresh-errors journey-refresh-errors
        :refreshing (hash-table-count journey-refreshing)
        :merge-queue (and (boundp 'jabber-mam-sync--merge-queue)
                          (mapcar #'buffer-name jabber-mam-sync--merge-queue))
        :room-buffers (delq nil (mapcar (lambda (b)
                                          (with-current-buffer b
                                            (and (bound-and-true-p jabber-group)
                                                 (list (buffer-name) major-mode
                                                       (and (fboundp 'jabber-mam-sync--buffer-key)
                                                            (jabber-mam-sync--buffer-key b))))))
                                        (buffer-list)))
        :session-pid (emacs-pid) :refused (and (boundp 'journey-refused) journey-refused)
        :jobs (journey-sync-jobs) :finish-errors journey-finish-errors))
(defvar journey-finish-errors nil "Errors the session's handling of a finished sync signalled.")
(with-eval-after-load 'jabber-mam-sync
  (advice-add 'jabber-mam-sync--finished :around
              (lambda (function &rest args)
                (condition-case err (apply function args)
                  (error (push (format "%S" err) journey-finish-errors) nil)))))
(defun journey-sync-jobs ()
  "Each account's job: whether its future resolved, its isolate lives, and a
look at a resolved result."
  (when (boundp 'jabber-mam-sync--accounts)
    (let (jobs)
      (maphash (lambda (account state)
                 (let ((future (plist-get state :future)))
                   (push (list account (and future t)
                               (and future (isolate-future-done-p future))
                               (and future (isolate-live-p (isolate-future-isolate future)))
                               (and future (isolate-future-done-p future)
                                    (substring (format "%S" (condition-case err (isolate-await future 0)
                                                              (error (list 'error err))))
                                               0 (min 300 (length (format "%S" (condition-case err (isolate-await future 0) (error (list 'error err))))))))
                               (plist-get state :pending) (and (plist-get state :timer) t) (plist-get state :jobs))
                         jobs)))
               jabber-mam-sync--accounts)
      jobs)))
'''

FIXTURE_ACCOUNT = r'''
(setq jabber-account-list
      `(("lane@fixture.test/journey" (:password . "fixture") (:network-server . "127.0.0.1")
         (:port . ,(string-to-number (getenv "JOURNEY_PORT"))) (:connection-type . network))))
(unless (getenv "JOURNEY_RESTRICTED")
  (setq jabber-muc-autojoin (split-string (getenv "JOURNEY_ROOMS") "," t)))
'''

REAL_ACCOUNT = r'''
(require 'auth-source-pass)
(setq auth-source-pass-filename (getenv "JOURNEY_PASS_STORE"))
;; gpg finds the user's own agent (and its cached key) through HOME.
(let* ((process-environment (cons (concat "HOME=" (getenv "JOURNEY_REAL_HOME")) process-environment))
       (password (auth-source-pass-get 'secret (getenv "JOURNEY_PASS_ENTRY"))))
  (unless (and (stringp password) (not (string-empty-p password)))
    (error "Jabber password unavailable"))
  (setq jabber-account-list `((,(getenv "JOURNEY_JID") (:password . ,password)))))
'''

# The live connection, read-only and quiet (--restricted, --account real).
RESTRICTED = r'''
(setq jabber-sm-enable nil
      jabber-muc-autojoin nil
      jabber-post-connect-hooks '(jabber-mam-maybe-catchup)
      jabber-post-resume-hooks nil
      jabber-lifecycle-session-bootstrap-functions
      '(jabber-presence--request-initial-roster jabber-bookmarks--prefetch))
(defvar journey-refused nil "What the live connection's gate refused, newest first.")
(defconst journey--reads
  '((query . "jabber:iq:roster") (query . "jabber:iq:private")
    (pubsub . "http://jabber.org/protocol/pubsub") (query . "http://jabber.org/protocol/disco#items")))
(defun journey--allowed-p (sexp)
  (or (jabber-mam-sync--allowed-p sexp)
      (and (eq (jabber-xml-node-name sexp) 'iq)
           (equal (jabber-xml-get-attribute sexp 'type) "get")
           (let ((children (jabber-xml-node-children sexp)))
             (and (null (cdr children))
                  (member (cons (jabber-xml-node-name (car children)) (jabber-xml-get-xmlns (car children)))
                          journey--reads))))))
(advice-add 'jabber-send-sexp--raw :before
            (lambda (_jc sexp)
              (unless (journey--allowed-p sexp)
                (push (prin1-to-string (jabber-xml-node-name sexp)) journey-refused)
                (signal 'jabber-mam-sync-refused (list (jabber-xml-node-name sexp))))))
(advice-add 'jabber-send-string :before
            (lambda (_jc string)
              (unless (jabber-mam-sync--allowed-string-p string)
                (push (substring string 0 (min 40 (length string))) journey-refused)
                (signal 'jabber-mam-sync-refused (list (substring string 0 (min 40 (length string))))))))
(defun journey-request-rooms ()
  "Catch up the rooms of JOURNEY_ROOMS, or else of the bookmarks, without joining them."
  (let ((jc (car jabber-connections)) (rooms (split-string (or (getenv "JOURNEY_ROOMS") "") "," t)))
    (if rooms
        (dolist (room rooms) (jabber-mam-sync-request jc room))
      (jabber-get-bookmarks
       jc (lambda (jc bookmarks)
            (dolist (bookmark bookmarks)
              (when-let* ((room (plist-get bookmark :jid)))
                (jabber-mam-sync-request jc room))))))
    t))
'''


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


KINDS = {'motion': ('next-line', 'previous-line'), 'self-insert-command': ('self-insert-command',),
         'select-text': ('journey-select-text',), 'select-notmuch': ('journey-select-notmuch',),
         'open-chat': ('journey-open-chat',), 'send': ('journey-send',)}


def schedule(duration, seed, chats=False):
    """(GAP, KEYS) steps: bursts of C-n/C-p in the notmuch window and of
    letters in the text window, F6/F5 (select that window) between, for
    DURATION seconds.  With CHATS, every third switch also shows the next
    chat (F7) and, when CHATS is `send', sends a message from it (F8)."""
    rng = random.Random(seed)
    steps, total, window, switches = [], 0.0, 'notmuch', 0
    while total < duration:
        switches += 1
        if chats and switches % 3 == 0:
            steps.append((1.0, (['-k', 'F7'], 'open-chat')))
            total += 1.0
            if chats == 'send':
                steps.append((1.0, (['-k', 'F8'], 'send')))
                total += 1.0
        burst = rng.randint(6, 16)
        for _ in range(burst):
            if window == 'notmuch':
                key = (['-M', 'ctrl', '-k', rng.choice('nnnp'), '-m', 'ctrl'], 'motion')
            else:
                key = ([rng.choice('etaoinshrdlu ')], 'self-insert-command')
            gap = rng.uniform(.11, .22)
            steps.append((gap, key))
            total += gap
        gap = rng.choice((.4, .7, 1.0, 1.5, 2.5))
        window = 'text' if window == 'notmuch' else 'notmuch'
        steps.append((gap, (['-k', 'F6' if window == 'text' else 'F5'], f'select-{window}')))
        total += gap
    return steps


def read_eld(path):
    """Each line of PATH, read as Lisp."""
    return [parity_live.read_sexp(line) for line in path.read_text(errors='replace').splitlines() if line.strip()]


def cpu_seconds(path):
    """utime + stime of a /proc stat file, in seconds."""
    fields = path.read_text().rsplit(')', 1)[1].split()
    return (int(fields[11]) + int(fields[12])) / os.sysconf('SC_CLK_TCK')


def evaluator_stat(pid):
    for task in Path(f'/proc/{pid}/task').iterdir():
        if (task / 'comm').read_text().strip().startswith('romacs-evaluat'):
            return task / 'stat'
    raise RuntimeError('no evaluator thread')


# The background sync's own work on the main VM.  The session's SQLite
# statements are the live stream's (a wait on the worker's lock among
# them): jabber's, reported apart in `sqlite_waits'.
SYNC_SPANS = ('isolate', 'jabber-mam-sync--start', 'jabber-mam-sync--finished', 'jabber-mam-sync--merge-next',
              'jabber-chat--insert-backlog-chunked', 'jabber-chat-buffer-refresh',
              'jabber-chat-buffer--clear-chunked', 'jabber-chat-buffer--reload',
              'jabber-chat-buffer--refresh-complete', 'jabber-mam-sync--refresh',
              'jabber-mam-sync--shown')


def covered(intervals, at, done):
    """Seconds of [AT, DONE] that INTERVALS (START END) cover: their union,
    so nested or overlapping spans count once."""
    clipped = sorted((max(at, s), min(done, e)) for s, e in intervals if s < done and at < e)
    total, end = 0.0, None
    for s, e in clipped:
        if end is None or s > end:
            total += e - s
            end = e
        elif e > end:
            total += e - end
            end = e
    return total


def attribute(typed, records, spans):
    """Per typed key: latency, collections since the previous command, and
    the jabber spans that overlapped it (association, not cause).
    RECORDS start with the state before the first key."""
    rows = []
    for i, (at, kind) in enumerate(typed):
        before, after = records[i], records[i + 1]
        done = after[4]
        overlap = [s for s in spans if s[0] < done and at < s[1]]
        rows.append({'kind': kind, 'at': at, 'latency_ms': round((done - at) * 1e3, 2), 'command': str(after[0]),
                     'gcs_since_previous': after[2] - before[2], 'gc_s': round(after[3] - before[3], 4),
                     'jabber_ms': round(covered([s[:2] for s in overlap], at, done) * 1e3, 2),
                     'sync_ms': round(covered([s[:2] for s in overlap if str(s[2]) in SYNC_SPANS], at, done) * 1e3, 2),
                     'jabber': sorted({str(s[2]) for s in overlap})})
    return rows


def attributable(row, field, slow):
    """Whether FIELD's work alone accounts for ROW's excess over SLOW: the
    key would have been under the limit without it.  Overlap alone is
    association; this is the counterfactual bound."""
    excess = row['latency_ms'] - slow * 1e3
    return excess > 0 and row[field] >= excess


def summary(rows, slow):
    latencies = sorted(r['latency_ms'] for r in rows)
    pick = lambda f: latencies[min(len(latencies) - 1, int(f * len(latencies)))]
    over = [r for r in rows if r['latency_ms'] > slow * 1e3]
    jabber = [r for r in over if r['jabber_ms'] > 0]
    sync = [r for r in over if r['sync_ms'] > 0]
    jabber_caused = [r for r in over if attributable(r, 'jabber_ms', slow)]
    sync_caused = [r for r in over if attributable(r, 'sync_ms', slow)]
    gc = [r for r in rows if r['gcs_since_previous']]
    return {'keys': len(rows), 'p50_ms': pick(.5), 'p99_ms': pick(.99), 'max_ms': latencies[-1],
            'over_slow': len(over), 'over_slow_associated_jabber': len(jabber),
            'over_slow_associated_jabber_worst_ms': max((r['latency_ms'] for r in jabber), default=0),
            'over_slow_associated_sync': len(sync),
            'over_slow_associated_sync_worst_ms': max((r['latency_ms'] for r in sync), default=0),
            'over_slow_attributable_jabber': len(jabber_caused),
            'over_slow_attributable_jabber_worst_ms': max((r['latency_ms'] for r in jabber_caused), default=0),
            'over_slow_attributable_sync': len(sync_caused),
            'over_slow_other_worst_ms': max((r['latency_ms'] for r in over if r['jabber_ms'] == 0), default=0),
            'keys_after_gc': len(gc), 'keys_after_gc_worst_ms': max((r['latency_ms'] for r in gc), default=0),
            'jabber_in_keys_ms': round(sum(r['jabber_ms'] for r in rows), 1)}


def span_stats(spans, begin, end):
    """Per span name within [BEGIN, END]: count, total and worst milliseconds."""
    stats = {}
    for start, stop, what in spans:
        if begin is not None and stop < begin or end is not None and start > end:
            continue
        entry = stats.setdefault(str(what), [0, 0.0, 0.0])
        entry[0] += 1
        entry[1] += (stop - start) * 1e3
        entry[2] = max(entry[2], (stop - start) * 1e3)
    return {k: {'count': v[0], 'total_ms': round(v[1], 1), 'worst_ms': round(v[2], 2)}
            for k, v in sorted(stats.items(), key=lambda kv: -kv[1][1])}


def sqlite_waits(statements, report):
    """The session's SQLite statements while the worker wrote the database
    and outside it: count, p50, p99 and worst milliseconds.  A statement
    that met the worker's transaction waited for it."""
    busy = []
    for account, summary_, finished in (report.get('jabber') or {}).get('summaries') or []:
        seconds = parity_live.plist(summary_).get('seconds')
        if seconds:
            busy.append((finished - seconds, finished))
    inside = sorted((e - s) * 1e3 for s, e, _ in statements if any(b <= s <= f for b, f in busy))
    outside = sorted((e - s) * 1e3 for s, e, _ in statements if not any(b <= s <= f for b, f in busy))
    pick = lambda v, f: round(v[min(len(v) - 1, int(f * len(v)))], 3) if v else None
    return {name: {'count': len(v), 'p50_ms': pick(v, .5), 'p99_ms': pick(v, .99), 'worst_ms': pick(v, 1)}
            for name, v in (('during_worker', inside), ('otherwise', outside))}


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--variant', choices=('session', 'worker', 'none'), default='session')
    parser.add_argument('--account', choices=('fixture', 'real'), default='fixture')
    parser.add_argument('--restricted', action='store_true', help='a read-only, quiet live connection')
    parser.add_argument('--push-bookmark', action='store_true',
                        help='the fixture pushes an autojoin bookmark event (the restricted mode must not join)')
    parser.add_argument('--real-jid', help="the real account, JID/RESOURCE (the resource must be the harness's own)")
    parser.add_argument('--real-pass-entry', help='its password-store entry, read by auth-source-pass in the session')
    parser.add_argument('--real-db', type=Path, help="a COPY of the account's jabber database to start from")
    parser.add_argument('--page', type=int, default=50)
    parser.add_argument('--backlog', type=int, default=30, help='jabber-backlog-number: messages a chat buffer shows')
    parser.add_argument('--rooms', default=ROOMS)
    parser.add_argument('--duration', type=float, default=60)
    parser.add_argument('--lead', type=float, default=1.0, help='seconds after the connect before typing')
    parser.add_argument('--wait', type=float, default=600, help='seconds to wait for the catch-up after typing')
    parser.add_argument('--slow', type=float, default=.016)
    parser.add_argument('--seed', type=int, default=1)
    parser.add_argument('--profile', action='store_true',
                        help='CPU-profile the session from connecting to the end (profile.txt)')
    parser.add_argument('--chats', choices=('open', 'send'),
                        help='every third window switch, show the next chat (F7); with send, also send '
                             'a message from it (F8, fixture only)')
    parser.add_argument('--tools', default=os.path.dirname(os.environ.get('ROMACS_TEST_LABWC', '/usr/bin/labwc')))
    parser.add_argument('--timeout', type=float, default=90)
    parser.add_argument('--resync-after', type=float, default=None,
                        help='seconds into the typing to request a second background catch-up (worker variant)')
    parser.add_argument('--base', type=Path, help='where the private tree goes (default XDG_RUNTIME_DIR, a tmpfs)')
    parser.add_argument('--keep', action='store_true', help='keep the private tree (HOME, world, database)')
    args = parser.parse_args()
    restricted = args.restricted or args.account == 'real'
    if args.chats == 'send' and restricted:
        parser.error('--chats send sends messages: fixture account, unrestricted, only')
    args.output.mkdir(parents=True, exist_ok=False)
    base = args.base or Path(os.environ.get('XDG_RUNTIME_DIR', '/tmp'))
    root, home, env = private_tree('jabber', base, args.tools)
    env.update(notmuch_fixture.build(home, ANCHOR))
    text = root / 'work' / 'notes.txt'
    text.write_text(''.join(f'Line {i}: some notes to type into.\n' for i in range(200)))
    env.update(JOURNEY_PAGE=str(args.page), JOURNEY_VARIANT=args.variant, JOURNEY_BACKLOG=str(args.backlog))
    if restricted:
        env['JOURNEY_RESTRICTED'] = '1'
    fixture, expected, init = None, [], INIT
    if args.variant != 'none':
        init += JABBER
        if args.account == 'fixture':
            port = free_port()
            command = [sys.executable, '-I', str(HARNESS / 'xmpp_fixture.py'), '--port', str(port),
                       '--rooms', args.rooms, '--anchor', str(ANCHOR),
                       '--log', str(args.output / 'fixture.jsonl'), '--stats', str(args.output / 'fixture-stats.json')]
            if args.push_bookmark:
                command += ['--push-bookmark', 'pushed@conference.fixture.test']
            fixture = subprocess.Popen(command, stdout=(args.output / 'fixture.out').open('w'), stderr=subprocess.STDOUT)
            rooms = [f"{spec.split(':')[0]}@conference.fixture.test" for spec in args.rooms.split(',')]
            env.update(JOURNEY_PORT=str(port), JOURNEY_ROOMS=','.join(rooms))
            expected = [None] + rooms
            init += FIXTURE_ACCOUNT
        else:
            if not (args.real_jid and args.real_pass_entry) or '/' not in args.real_jid:
                parser.error('--account real needs --real-jid JID/RESOURCE and --real-pass-entry')
            env.update(JOURNEY_JID=args.real_jid, JOURNEY_PASS_ENTRY=args.real_pass_entry,
                       JOURNEY_PASS_STORE=str(Path.home() / '.password-store'), JOURNEY_REAL_HOME=str(Path.home()))
            init += REAL_ACCOUNT
            expected = [None]
            if args.real_db:
                (home / 'jabber.db').write_bytes(args.real_db.read_bytes())
        if restricted:
            init += RESTRICTED
    init += (f'\n(random "jabber-journey")\n(load {lisp_string(str(parity_live.PROBE))} nil t)\n'
             '(with-temp-file (expand-file-name "alive" parity-probe-dir) (insert (number-to-string (float-time))))\n')
    report = {'binary': str(args.binary), 'variant': args.variant, 'account': args.account, 'restricted': restricted,
              'page': args.page, 'backlog': args.backlog, 'duration': args.duration, 'problems': []}
    expected_lisp = '(list ' + ' '.join('nil' if a is None else lisp_string(a) for a in expected) + ')'
    engine = romacs_engine(str(args.binary.resolve()), root, home, env, ';; -*- lexical-binding: t -*-\n' + init,
                           None, args.timeout, args.tools)
    records_before, typed, first = 0, [], None
    try:
        report['windows'] = engine.ask(f'(journey-windows {lisp_string(str(text))})', timeout=120)
        time.sleep(2)
        pid = engine.process.pid
        stat = evaluator_stat(pid)
        if args.profile:
            engine.ask("(progn (profiler-start 'cpu) t)")
        connect_at = time.time()
        report['clock'] = {'wall': time.time(), 'monotonic_ns': time.monotonic_ns()}
        if args.variant != 'none':
            asked = time.time()
            engine.ask('(progn (jabber-connect-all) t)', timeout=60)
            report['clock']['connect_ask_s'] = round(time.time() - asked, 3)
            deadline = time.time() + 60
            while time.time() < deadline and not engine.ask(
                    '(and jabber-connections (eq (get (car jabber-connections) :state) :session-established) t)'):
                time.sleep(.5)
            if restricted:
                engine.ask('(journey-request-rooms)', timeout=60)
            elif args.account == 'fixture':
                asked = time.time()
                engine.ask('(journey-open-rooms)', timeout=60)
                report['clock']['open_rooms_ask_s'] = round(time.time() - asked, 3)
        time.sleep(args.lead)
        cpu0, start = cpu_seconds(stat), time.time()
        records_before = engine.ask('(length journey-records)')
        # The state before the first key, as the record of a command would hold it.
        first = engine.ask('(list (quote start) nil gcs-done gc-elapsed (float-time))')
        resync = args.resync_after is not None and args.variant == 'worker'
        for gap, (keys, kind) in schedule(args.duration, args.seed, args.chats):
            if resync and time.time() - start >= args.resync_after:
                # Through the session's own timer, as a later request would come.
                engine.ask('(progn (run-with-timer 0 nil (function journey-resync)) t)')
                resync = False
            typed.append((time.time(), kind))
            engine.keys(keys)
            time.sleep(gap)
        time.sleep(1)
        cpu1, end = cpu_seconds(stat), time.time()
        report['typing'] = {'seconds': round(end - start, 3), 'evaluator_cpu_s': round(cpu1 - cpu0, 3),
                            'evaluator_cpu_share': round((cpu1 - cpu0) / (end - start), 3)}
        if args.variant != 'none':
            state_form = f'(journey-jabber-state {expected_lisp})'
            deadline = time.time() + args.wait
            while time.time() < deadline and not parity_live.plist(engine.ask(state_form)).get('done'):
                time.sleep(2)
            state = parity_live.plist(engine.ask(state_form))
            report['jabber'] = state
            done = state.get('done')
            report['jabber']['catch_up_s'] = round(done - connect_at, 3) if done else None
            if not done:
                report['problems'].append(f'catch-up not finished after {args.wait:g} s of waiting')
            if args.variant == 'worker' and args.account == 'fixture' and not restricted and not args.chats:
                # The rooms' buffers are hidden: their history waits, and loads once shown.
                # (With --chats, F7 has shown them already.)
                stale = engine.ask('(journey-stale-buffers)') or []
                name, shown_at = engine.ask('(journey-show-room)')
                deadline = time.time() + 60
                while time.time() < deadline and engine.ask('(hash-table-count journey-refreshing)'):
                    time.sleep(.5)
                report['shown_room'] = {'stale_before': stale, 'room': name,
                                        'nodes': engine.ask(f'(journey-room-nodes {lisp_string(name)})'),
                                        'stale_after': engine.ask('(journey-stale-buffers)') or [],
                                        'filled_s': round(time.time() - shown_at, 2)}
                if name not in stale or name in report['shown_room']['stale_after']:
                    report['problems'].append(f'room buffer {name} was not stale until shown: {report["shown_room"]}')
        if args.chats:
            stored, live, live_distinct, shown = engine.ask('(journey-duplicates)', timeout=60)
            posts = engine.ask('(reverse journey-posts)') or []
            report['chats'] = {'sent': engine.ask('journey-sent'), 'stored_duplicates': stored,
                               'live_rows': live, 'live_distinct': live_distinct, 'shown_duplicates': shown,
                               'posts': len(posts),
                               'first_post_s': round(posts[0][0] - connect_at, 3) if posts else None}
            if stored or shown or live != live_distinct:
                report['problems'].append(f'duplicates after the handover: {report["chats"]}')
        report['evaluator_cpu_total_s'] = round(cpu_seconds(stat), 3)
        report['window_buffers'] = engine.ask('(journey-window-buffers)')
        if args.profile:
            engine.ask(f'(journey-profile-dump {lisp_string(str(args.output / "profile.txt"))})', timeout=120)
        engine.ask(f'(journey-dump {lisp_string(str(args.output))})', timeout=60)
    finally:
        engine.stop()
        if fixture:
            fixture.terminate()
            fixture.wait(10)
        if not args.keep:
            # Its world and database copy are hundreds of megabytes.
            shutil.rmtree(root, ignore_errors=True)
    check_jabber(report, args, restricted)
    records = [first] + read_eld(args.output / 'records.eld')[records_before:]
    spans = read_eld(args.output / 'spans.eld') + [s[:2] + ['sqlite'] for s in read_eld(args.output / 'sqlite.eld')]
    spans += [r[:2] + ['jabber-chat-buffer-refresh'] for r in (report.get('jabber') or {}).get('refreshes') or []]
    # The ticks run from timers, already spans; the other steps are not.
    spans += [r[:2] + [str(r[2])] for r in read_eld(args.output / 'refresh-phases.eld')
              if str(r[2]) not in ('jabber-chat--insert-backlog-chunked', 'jabber-chat-buffer--clear-chunked')]
    if len(records) != len(typed) + 1:
        report['problems'].append(f'{len(records) - 1} commands recorded for {len(typed)} typed commands')
    else:
        unpaired = [(i, kind, str(r[0])) for i, ((_, kind), r) in enumerate(zip(typed, records[1:]))
                    if str(r[0]) not in KINDS[kind]]
        if unpaired:
            report['problems'].append(f'{len(unpaired)} keys ran another command than typed: {unpaired[:5]}')
        rows = attribute(typed, records, spans)
        report['summary'] = summary(rows, args.slow)
        window = report.get('jabber') or {}
        begin, finish = window.get('start'), window.get('done')
        if begin:
            during = [r for r in rows if r['at'] >= begin and (finish is None or r['at'] <= finish)]
            report['catch_up_window'] = {'start': begin, 'end': finish, 'keys': len(during),
                                         'summary': summary(during, args.slow) if during else None,
                                         'spans': span_stats(spans, begin, finish)}
            if args.variant == 'worker' and during and report['catch_up_window']['summary']['over_slow_attributable_sync']:
                report['problems'].append('keys typed during the catch-up were over the limit because of the sync\'s work')
        report['spans'] = span_stats(spans, None, None)
        report['sqlite_waits'] = sqlite_waits(read_eld(args.output / 'sqlite.eld'), report)
        report['slow_keys'] = [r for r in rows if r['latency_ms'] > args.slow * 1e3]
        report['rows'] = rows
    report['status'] = 'PASS' if report.get('summary') and not report['problems'] else 'FAIL'
    (args.output / 'report.json').write_text(json.dumps(report, indent=1, default=str) + '\n')
    shown = {k: report.get(k) for k in ('status', 'problems', 'summary', 'typing')}
    shown['catch_up_window'] = {k: v for k, v in (report.get('catch_up_window') or {}).items() if k != 'spans'}
    shown['jabber'] = {k: v for k, v in (report.get('jabber') or {}).items() if k not in ('summaries', 'outcomes')}
    print(json.dumps(shown, indent=1, default=str))
    return 0 if report['status'] == 'PASS' else 1


def check_jabber(report, args, restricted):
    """The catch-up's own failures: failed archives, no worker, a client
    presence or message seen by the fixture in the restricted mode."""
    state = report.get('jabber') or {}
    for account, summary_, _ in state.get('summaries') or []:
        summary_ = parity_live.plist(summary_)
        report.setdefault('sync', []).append({k: v for k, v in summary_.items() if k != 'peers'})
        if summary_.get('error'):
            report['problems'].append(f'background sync of {account} failed: {summary_["error"]}')
        for archive in summary_.get('archives') or []:
            # An archive the server says it does not keep is nothing to read.
            if str(archive[2]) not in ('success', 'unsupported'):
                report['problems'].append(f'archive {archive[0] or "personal"} settled {archive[2]} ({archive[4]})')
        if summary_.get('refused'):
            report['problems'].append(f'the worker\'s gate refused {summary_["refused"]}')
        if not summary_.get('pid') or summary_.get('pid') == state.get('session-pid'):
            report['problems'].append('the background sync ran in no worker')
    if state.get('refresh-errors'):
        report['problems'].append(f'buffer refreshes signalled: {state["refresh-errors"][:3]}')
    if state.get('refreshing'):
        report['problems'].append(f'{state["refreshing"]} buffer refreshes never finished')
    if args.variant == 'worker' and not state.get('summaries'):
        report['problems'].append('no background sync finished')
    for entry in state.get('outcomes') or []:
        # (ARCHIVE . OUTCOME), read as [ARCHIVE, '.', OUTCOME].
        archive, outcome = entry[0], entry[-1]
        if str(outcome) != 'success':
            report['problems'].append(f'in-session query of {archive or "personal"} settled {outcome}')
    chats = report.get('chats')
    stats_file = args.output / 'fixture-stats.json'
    if chats and args.account == 'fixture' and not restricted and stats_file.exists():
        received = json.loads(stats_file.read_text()).get('client_messages', 0)
        chats['fixture_received'] = received
        if received < (chats.get('sent') or 0):
            report['problems'].append(f'sent {chats["sent"]} messages, the fixture received {received}')
    if restricted:
        refused = state.get('refused') or []
        report['refused'] = refused
        # A pushed autojoin bookmark makes jabber try to join: the gate's
        # refusal of that presence is the expected outcome.
        unexpected = [r for r in refused if not (args.push_bookmark and r == 'presence')]
        if unexpected:
            report['problems'].append(f'the live connection\'s gate refused {unexpected}')
        stats_file = args.output / 'fixture-stats.json'
        if args.account == 'fixture' and stats_file.exists():
            stats = json.loads(stats_file.read_text())
            for key in ('client_presences', 'client_messages'):
                if stats.get(key):
                    report['problems'].append(f'the fixture saw {stats[key]} {key.replace("_", " ")}')


if __name__ == '__main__':
    sys.exit(main())
