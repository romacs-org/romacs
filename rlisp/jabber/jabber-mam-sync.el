;;; jabber-mam-sync.el --- Archive catch-up in a worker VM  -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Thanos Apollo

;; Author: Thanos Apollo <public@thanosapollo.org>
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; This file is part of emacs-jabber.

;; emacs-jabber is free software: you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.

;; emacs-jabber is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.

;; You should have received a copy of the GNU General Public License
;; along with emacs-jabber.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:

;; The catch-up of the personal archive, of rooms and of chats
;; (jabber-mam.el) runs in a worker VM (Romacs's isolates,
;; rlisp/vm/isolate.el) over an XMPP session of its own, so a long
;; catch-up never holds the editor: the worker reads the socket, parses
;; the stanzas, pages the archives and writes the database.  The session
;; it opens is quiet and read-only: a resource of its own, no presence,
;; no room joined, and a send gate that passes only stream negotiation,
;; disco#info queries and archive queries.
;;
;; The live session only collects the archives to catch up, starts one
;; job per account, and when the job ends shows one toast and merges the
;; new history into open chat buffers, one buffer per idle moment.
;; Where no worker can start from the session's world (batch, a VM
;; booted from source), the catch-up runs in the session as before.

;;; Code:

(require 'cl-lib)
(require 'jabber-mam)
(require 'jabber-db)
(require 'jabber-disco)
(require 'jabber-chatbuffer)

(declare-function jabber-connect "jabber-core")
(declare-function jabber-disconnect-one "jabber-core")
(declare-function isolate-spawn "isolate")
(declare-function isolate-on-done "isolate")
(declare-function isolate-await "isolate")
(declare-function isolate-future-isolate "isolate")
(declare-function isolate-future-cancel "isolate")
(declare-function isolate-retire "isolate")
(declare-function isolate-live-p "isolate")
(declare-function romacs-layer-toast "romacs-layer")
(declare-function thaw "isolate" (object))
(defvar isolate-boot)
(defvar isolate-world)
(defvar jabber-connections)
(defvar jabber-post-connect-hooks)
(defvar jabber-post-resume-hooks)
(defvar jabber-auto-reconnect)
(defvar jabber-debug-log-xml)
(defvar jabber-chat--crypto-loaded)
(defvar jabber-sm-enable)
(defvar jabber-buffer-connection)
(defvar jabber-message-thread-id)
(defvar jabber-message-thread-peer)
(defvar jabber-message-thread-type)
(defvar jabber-group)
(defvar jabber-chatting-with)
(defvar jabber-chat--loaded-at)
(declare-function jabber-chat-buffer-refresh "jabber-chatbuffer")
(declare-function jabber-db-mark-message-thread-read "jabber-db")
(declare-function auth-source-search "auth-source")

;;; Customization

(defcustom jabber-mam-background t
  "Whether archive catch-ups run in a worker VM, out of the editor's way.
This needs a worker that starts from the session's world, as Romacs's
GUI sessions have; elsewhere the catch-up runs in the session."
  :type 'boolean
  :group 'jabber)

(defcustom jabber-mam-sync-delay 3
  "Seconds after the last request before a background catch-up starts.
A connect and the rooms its autojoin enters make one job."
  :type 'number
  :group 'jabber)

(defcustom jabber-mam-sync-timeout 120
  "Seconds without progress after which a background catch-up stops.
Progress is a committed archived message or a settled page.  A full
catch-up of a long history takes as long as the server's paging needs;
archives left unsettled are reported incomplete, and their cursors let
the next catch-up continue where this one stopped."
  :type 'number
  :group 'jabber)

(defcustom jabber-mam-sync-parallel 3
  "Archives a background catch-up reads at once."
  :type 'natnum
  :group 'jabber)

(defcustom jabber-mam-sync-slices 4
  "Queries a background catch-up of the personal archive runs at once.
A long catch-up is split by time into slices paged side by side; a
slice that finishes takes the later half of the widest one left."
  :type 'natnum
  :group 'jabber)

(defvar jabber-mam-sync-min-slice 600
  "Seconds of archive a slice keeps at least when it is split.")

(defcustom jabber-mam-sync-page-size 250
  "Results a background catch-up's page asks for.
Servers clamp a page to their own maximum.  A page's results are worked
in the worker, so a large page only saves round trips."
  :type 'natnum
  :group 'jabber)

(defvar jabber-mam-sync-inherited-variables
  '(jabber-db-path jabber-db-busy-timeout jabber-mam-page-size
    jabber-mam-catch-up-min-page-size jabber-mam-room-history
    jabber-mam-sync-timeout
    jabber-mam-sync-parallel jabber-mam-sync-page-size jabber-mam-sync-slices
    jabber-default-connection-type
    network-security-level nsm-settings-file gnutls-verify-error
    gnutls-trustfiles gnutls-min-prime-bits gnutls-algorithm-priority)
  "Variables whose session values a background catch-up's worker gets.")

(defvar jabber-mam-sync-finished-functions nil
  "Abnormal hook run when a background catch-up ends.
Each function receives the account's bare JID and the job's summary,
as `jabber-mam-sync-worker-run' returns it, or (:error REASON).")

;;; Session side: requests and jobs

(defvar jabber-mam-sync--accounts (make-hash-table :test #'equal)
  "Per account: BARE-JID to STATE.
STATE is a plist: :jc, the account's live connection; :pending, the
archives to catch up, each (ROOM . WITH), nil ROOM for the personal
archive, newest first; :index, a hash of the pending ones; :timer, when
its delayed start is due (`jabber-mam-sync--due'); :future, the running job;
:cancelling, cancelled jobs whose worker has not answered yet; :jobs,
how many jobs have run since the account connected.  An account starts
no job while one runs or is being cancelled, so two workers never
catch up the same account at once.")

(defun jabber-mam-sync-available-p ()
  "Non-nil when a catch-up can run in a worker started from this world."
  (and jabber-mam-background
       (fboundp 'isolate-spawn)
       (boundp 'isolate-boot) (eq isolate-boot 'world)
       (boundp 'isolate-world) isolate-world))

(defun jabber-mam-sync--get (account key)
  "KEY of ACCOUNT's sync state."
  (plist-get (gethash account jabber-mam-sync--accounts) key))

(defun jabber-mam-sync--put (account key value)
  "Set KEY of ACCOUNT's sync state to VALUE; return VALUE."
  (puthash account (plist-put (gethash account jabber-mam-sync--accounts) key value)
           jabber-mam-sync--accounts)
  value)

(defun jabber-mam-sync--forget-if-idle (account)
  "Drop ACCOUNT's state once it has no connection, job or request."
  (unless (or (jabber-mam-sync--get account :jc) (jabber-mam-sync--busy-p account)
              (jabber-mam-sync--get account :pending))
    (remhash account jabber-mam-sync--accounts)))

(defvar jabber-mam-sync--live-jobs 0
  "Jobs spawned whose worker has not answered yet, cancelled ones included.")

(defun jabber-mam-sync--any-busy-p ()
  "Non-nil while any account has a job running or being cancelled."
  (> jabber-mam-sync--live-jobs 0))

(defun jabber-mam-sync--take-pending (account)
  "ACCOUNT's pending archives, oldest first, now no longer pending."
  (prog1 (reverse (jabber-mam-sync--get account :pending))
    (jabber-mam-sync--put account :pending nil)
    (jabber-mam-sync--put account :index nil)))

(defun jabber-mam-sync--busy-p (account)
  "Non-nil while ACCOUNT has a job running or being cancelled."
  (or (jabber-mam-sync--get account :future)
      (jabber-mam-sync--get account :cancelling)))

(defun jabber-mam-sync-request (jc &optional room with)
  "Catch up an archive of JC in the background.
ROOM is a room's JID, nil for the personal archive; WITH a peer's bare
JID to read the personal archive's history with that peer only.  The
job starts `jabber-mam-sync-delay' seconds after the last request, or
after the account's running job ends."
  (let ((account (jabber-connection-bare-jid jc))
        (archive (cons room with)))
    (jabber-mam-sync--put account :jc jc)
    (let ((index (or (jabber-mam-sync--get account :index)
                     (jabber-mam-sync--put account :index (make-hash-table :test #'equal)))))
      (unless (gethash archive index)
        (puthash archive t index)
        (jabber-mam-sync--put account :pending
                              (cons archive (jabber-mam-sync--get account :pending)))))
    (unless (jabber-mam-sync--busy-p account)
      (jabber-mam-sync--arm account))))

(defvar jabber-mam-sync--due nil
  "Accounts to start, oldest due first: a queue of (DUE . ACCOUNT).
Every request is due `jabber-mam-sync-delay' after it is made, so
appending keeps the order; an entry whose DUE is no longer its
account's :timer is stale and skipped.")

(defvar jabber-mam-sync--due-tail nil
  "The last cons of `jabber-mam-sync--due'.")

(defvar jabber-mam-sync--timer nil
  "The one timer that starts due accounts' jobs, or nil.")

(defun jabber-mam-sync--arm (account)
  "Start ACCOUNT's job `jabber-mam-sync-delay' seconds from now."
  (let* ((due (+ (float-time) jabber-mam-sync-delay))
         (cell (list (cons due account))))
    (jabber-mam-sync--put account :timer due)
    (if jabber-mam-sync--due
        (setcdr jabber-mam-sync--due-tail cell)
      (setq jabber-mam-sync--due cell))
    (setq jabber-mam-sync--due-tail cell)
    (unless jabber-mam-sync--timer
      (setq jabber-mam-sync--timer
            (run-with-timer jabber-mam-sync-delay nil #'jabber-mam-sync--fire)))))

(defun jabber-mam-sync--fire ()
  "Start every account now due; wait for the next one."
  (setq jabber-mam-sync--timer nil)
  (let ((now (float-time)))
    (while (and jabber-mam-sync--due (<= (caar jabber-mam-sync--due) now))
      (pcase-let ((`(,due . ,account) (pop jabber-mam-sync--due)))
        (when (eql due (jabber-mam-sync--get account :timer))
          (jabber-lifecycle--call-contained #'jabber-mam-sync--start account))))
    (if jabber-mam-sync--due
        (setq jabber-mam-sync--timer
              (run-with-timer (max 0 (- (caar jabber-mam-sync--due) now)) nil
                              #'jabber-mam-sync--fire))
      (setq jabber-mam-sync--due-tail nil))))

(defun jabber-mam-sync--password (jc)
  "The password JC authenticated with, or nil.
The connection's own when it was given one, else what the auth source
holds for it (`jabber-read-password' without its prompt): a worker can
never ask."
  (let ((data (fsm-get-state-data jc)))
    (or (plist-get data :password)
        (ignore-errors
          (when-let* ((found (car (auth-source-search
                                   :user (plist-get data :username)
                                   :host (plist-get data :server)
                                   :port "xmpp" :max 1 :require '(:secret))))
                      (secret (plist-get found :secret)))
            (copy-sequence (if (functionp secret) (funcall secret) secret)))))))

(defun jabber-mam-sync--account (jc password)
  "What JC's worker needs to open a session of its own, as a plist.
The resource is JC's with a random suffix, so the live session is never
replaced; PASSWORD is the one JC authenticated with."
  (let ((data (fsm-get-state-data jc)))
    (list :username (plist-get data :username)
          :server (plist-get data :server)
          :resource (format "%s-sync-%08x" (or (plist-get data :resource) "jabber")
                            (random #xffffffff))
          :password password
          :network-server (plist-get data :network-server)
          :port (plist-get data :port)
          :connection-type (plist-get data :connection-type)
          :proxy (plist-get data :proxy))))

(defun jabber-mam-sync--archives (jc pending)
  "PENDING, oldest first, as the worker's archive specs for JC.
Rooms carry our nickname there."
  (mapcar (lambda (archive)
            (let ((room (car archive)))
              (list :room room :with (cdr archive)
                    :nick (and room (jabber-muc-nickname room jc)))))
          pending))

(defun jabber-mam-sync--bindings ()
  "The inherited variables' values, as `isolate-spawn' :bindings."
  (cl-loop for variable in jabber-mam-sync-inherited-variables
           when (boundp variable)
           collect (cons variable (symbol-value variable))))

(defun jabber-mam-sync--fail (account reason)
  "Report that ACCOUNT's background catch-up could not run, for REASON.
The catch-up does not fall back to the session: that is what the
background sync keeps the editor from."
  (let ((summary (list :error reason :archives nil :peers nil)))
    (jabber-mam-sync--toast (jabber-mam-sync--toast-text summary t) 'error)
    (jabber-lifecycle--dispatch-contained
     'jabber-mam-sync-finished-functions account summary)))

(defun jabber-mam-sync--start (account)
  "Start ACCOUNT's background catch-up of its pending archives."
  (jabber-mam-sync--put account :timer nil)
  (let ((jc (jabber-mam-sync--get account :jc))
        (pending (jabber-mam-sync--get account :pending)))
    ;; A connection that ends clears :jc (`jabber-mam-sync--cancel').
    (when (and pending jc (eq (get jc :state) :session-established)
               (not (jabber-mam-sync--busy-p account)))
      (let ((password (jabber-mam-sync--password jc))
            (archives (jabber-mam-sync--archives jc (jabber-mam-sync--take-pending account))))
        (if (null password)
            (jabber-mam-sync--fail account "no stored password for the account")
          (condition-case err
              (let ((future (isolate-spawn 'jabber-mam-sync-worker-run
                                           (jabber-mam-sync--account jc password) archives
                                           :require '(jabber) :fresh t
                                           :name "jabber-mam-sync"
                                           :bindings (jabber-mam-sync--bindings))))
                (jabber-mam-sync--put account :future future)
                (setq jabber-mam-sync--live-jobs (1+ jabber-mam-sync--live-jobs))
                (jabber-db-share-writes 'announce)
                (isolate-on-done future (lambda (f) (jabber-mam-sync--finished account f))))
            (error
             (jabber-mam-sync--fail
              account (format "no worker (%s)" (error-message-string err))))))))))

(defun jabber-mam-sync--retire (future)
  "Let FUTURE's settled worker end, without waiting for it.
A worker still answering an earlier frame is asked again shortly."
  (let ((isolate (isolate-future-isolate future)))
    (when (and (isolate-live-p isolate) (not (isolate-retire isolate)))
      (run-with-timer 1 nil #'jabber-mam-sync--retire future))))

(defun jabber-mam-sync--finished (account future)
  "FUTURE, ACCOUNT's job, settled: report it unless it was cancelled.
Then start the account's follow-up job, if anything is pending."
  (jabber-mam-sync--retire future)
  (setq jabber-mam-sync--live-jobs (max 0 (1- jabber-mam-sync--live-jobs)))
  (cond
   ((memq future (jabber-mam-sync--get account :cancelling))
    (jabber-mam-sync--put account :cancelling
                          (delq future (jabber-mam-sync--get account :cancelling))))
   ((eq future (jabber-mam-sync--get account :future))
    (jabber-mam-sync--put account :future nil)
    (jabber-mam-sync--report account future)))
  (unless (jabber-mam-sync--any-busy-p)
    (jabber-db-share-writes nil))
  (if (and (jabber-mam-sync--get account :pending)
           (not (jabber-mam-sync--busy-p account)))
      (jabber-mam-sync--arm account)
    (jabber-mam-sync--forget-if-idle account)))

(defun jabber-mam-sync--report (account future)
  "Toast ACCOUNT's job FUTURE's outcome, merge its history, run the hook."
  (let* ((summary (condition-case err
                      (thaw (isolate-await future 0))
                    (quit (list :error "cancelled" :archives nil :peers nil))
                    (error (list :error (error-message-string err)
                                 :archives nil :peers nil))))
         (jobs (1+ (or (jabber-mam-sync--get account :jobs) 0))))
    (jabber-mam-sync--put account :jobs jobs)
    (when-let* ((text (jabber-mam-sync--toast-text summary (= jobs 1))))
      (jabber-mam-sync--toast text (jabber-mam-sync--toast-kind summary)))
    ;; What the worker's watchdog did, for the record, without echoing.
    (dolist (stall (plist-get summary :stalls))
      (let ((inhibit-message t))
        (message "Jabber: sync watchdog: %s %s %S" (nth 0 stall) (nth 1 stall) (nth 2 stall))))
    (jabber-mam-sync--merge summary)
    (jabber-lifecycle--dispatch-contained
     'jabber-mam-sync-finished-functions account summary)))

(defun jabber-mam-sync--cancel (jc)
  "Stop the background catch-up of JC's account and forget its requests.
The running job is interrupted; until its worker answers, the account
starts no other job."
  (let ((account (ignore-errors (jabber-connection-bare-jid jc))))
    (when (and account (gethash account jabber-mam-sync--accounts)
               (memq (jabber-mam-sync--get account :jc) (list jc nil)))
      (jabber-mam-sync--stop account))))

(defun jabber-mam-sync--stop (account)
  "Interrupt ACCOUNT's job and drop its requests."
  (jabber-mam-sync--put account :timer nil)
  (jabber-mam-sync--take-pending account)
  (jabber-mam-sync--put account :jc nil)
  (jabber-mam-sync--put account :jobs 0)
  (when-let* ((future (jabber-mam-sync--get account :future)))
    (jabber-mam-sync--put account :future nil)
    (jabber-mam-sync--put account :cancelling
                          (cons future (jabber-mam-sync--get account :cancelling)))
    (condition-case nil
        (isolate-future-cancel future)
      (error nil)))
  (jabber-mam-sync--forget-if-idle account))

(defun jabber-mam-sync--cancel-all ()
  "Stop every background catch-up."
  (mapc #'jabber-mam-sync--stop (hash-table-keys jabber-mam-sync--accounts)))

;;; Session side: what the user sees

(defun jabber-mam-sync--plural (n word)
  "N WORDs, singular for one."
  (format "%d %s%s" n word (if (= n 1) "" "s")))

(defun jabber-mam-sync--toast-text (summary first)
  "The toast for SUMMARY, or nil when there is nothing to say.
FIRST is non-nil for the account's first job since it connected."
  (let* ((failure (plist-get summary :error))
         (archives (plist-get summary :archives))
         (failed (cl-remove-if-not (lambda (a) (eq (nth 2 a) 'failed)) archives))
         (incomplete (cl-remove-if-not (lambda (a) (eq (nth 2 a) 'incomplete)) archives))
         (still (mapconcat (lambda (a) (or (car a) "personal archive")) incomplete ", "))
         (peers (cl-remove-if-not (lambda (p) (> (nth 3 p) 0)) (plist-get summary :peers)))
         (messages (apply #'+ (mapcar (lambda (p) (nth 3 p)) peers)))
         (done (format "sync completed (%s in %s)"
                       (jabber-mam-sync--plural messages "message")
                       (jabber-mam-sync--plural (length peers) "chat")))
         (names (mapconcat (lambda (a) (or (car a) "personal archive")) failed ", ")))
    (cond (failure (format "Jabber: sync failed: %s" failure))
          ((and failed (= (length failed) (length archives)))
           (format "Jabber: sync failed for %s" names))
          ((or failed incomplete)
           (concat "Jabber: " done
                   (and failed (format "; failed for %s" names))
                   (and incomplete (format "; %s continues next sync" still))))
          ((or first (> messages 0)) (format "Jabber: %s" done)))))

(defun jabber-mam-sync--toast-kind (summary)
  "`error' when SUMMARY reports a failure, else nil."
  (and (or (plist-get summary :error)
           (cl-some (lambda (a) (eq (nth 2 a) 'failed)) (plist-get summary :archives)))
       'error))

(defun jabber-mam-sync--toast (text &optional kind)
  "Show TEXT once, as a toast of KIND where Romacs has them.
KIND `error' marks a failure, as `romacs-layer-toast' draws one."
  (if (fboundp 'romacs-layer-toast)
      (romacs-layer-toast text kind)
    (let ((message-log-max nil)) (message "%s" text))))

(defvar jabber-mam-sync--changed (make-hash-table :test #'equal)
  "History key (ACCOUNT PEER KIND) to when a catch-up last changed it.
KIND is \"groupchat\" or \"chat\".  A chat buffer whose history changed
after it loaded (`jabber-chat--loaded-at') is stale: it is refreshed
when it is shown, or, if shown already, at the next idle moment.  No
buffer is looked for: a merge costs its changed peers and the windows.")

(defvar jabber-mam-sync--merge-queue nil
  "Shown chat buffers to refresh with their new history, oldest first.")

(defvar jabber-mam-sync--merge-tail nil
  "The last cons of `jabber-mam-sync--merge-queue'.")

(defvar jabber-mam-sync--merge-queued (make-hash-table :test #'eq :weakness 'key)
  "The buffers of `jabber-mam-sync--merge-queue', for membership.")

(defvar jabber-mam-sync--merge-timer nil
  "The timer of `jabber-mam-sync--merge-next', or nil.")

(defvar jabber-mam-sync-merge-idle 0.5
  "Seconds without input before a shown buffer gets its new history.")

(defun jabber-mam-sync--stale-p (buffer)
  "Non-nil when a catch-up changed BUFFER's history after it loaded."
  (when-let* ((key (and (buffer-live-p buffer) (jabber-chat-buffer--history-key buffer)))
              (changed (gethash (butlast key) jabber-mam-sync--changed)))
    (> changed (or (buffer-local-value 'jabber-chat--loaded-at buffer) 0))))

(defun jabber-mam-sync--queue (buffer)
  "Queue shown BUFFER for its idle refresh, once."
  (unless (gethash buffer jabber-mam-sync--merge-queued)
    (puthash buffer t jabber-mam-sync--merge-queued)
    (let ((cell (list buffer)))
      (if jabber-mam-sync--merge-queue
          (setcdr jabber-mam-sync--merge-tail cell)
        (setq jabber-mam-sync--merge-queue cell))
      (setq jabber-mam-sync--merge-tail cell))))

(defun jabber-mam-sync--shown-stale (frames)
  "The stale chat buffers the windows of FRAMES show, thread buffers first.
A thread's refresh marks it read, which its parent's summary shows."
  (let (threads parents)
    (dolist (frame frames)
      (dolist (window (window-list frame 'never))
        (let ((buffer (window-buffer window)))
          (when (jabber-mam-sync--stale-p buffer)
            (if (with-current-buffer buffer (bound-and-true-p jabber-message-thread-id))
                (push buffer threads)
              (push buffer parents))))))
    (delete-dups (nconc (nreverse threads) (nreverse parents)))))

(defun jabber-mam-sync--queue-shown ()
  "Queue the stale buffers any window shows, for an idle refresh."
  (mapc #'jabber-mam-sync--queue (jabber-mam-sync--shown-stale (frame-list)))
  (when jabber-mam-sync--merge-queue
    (jabber-mam-sync--merge-arm)))

(defun jabber-mam-sync--merge (summary)
  "Record the histories SUMMARY changed; queue the shown buffers among them.
Hidden buffers are refreshed when shown (`jabber-mam-sync--shown').
The other functions of `jabber-mam-sync-complete-functions' run now,
once, with every changed peer."
  (let ((peers (cl-loop for (account peer type) in (plist-get summary :peers)
                        collect (list account peer type)))
        (now (float-time)))
    (when peers
      (dolist (peer peers)
        (puthash (list (nth 0 peer) (nth 1 peer)
                       (jabber-chat-buffer--history-kind (nth 2 peer)))
                 now jabber-mam-sync--changed))
      (jabber-mam-sync--queue-shown)
      (let ((jabber-mam-sync-complete-functions
             (remq #'jabber-chat--handle-mam-sync-complete
                   jabber-mam-sync-complete-functions)))
        (jabber-lifecycle--dispatch-contained 'jabber-mam-sync-complete-functions peers)))))

(defun jabber-mam-sync--merge-arm ()
  "Look for an idle moment soon."
  (unless jabber-mam-sync--merge-timer
    (setq jabber-mam-sync--merge-timer
          (run-with-timer 0.2 nil #'jabber-mam-sync--merge-next))))

(defun jabber-mam-sync--refresh (buffer)
  "Reload stale BUFFER's history from the database, as a completed catch-up does.
A visible thread buffer's thread is read, as the completion refresh of
`jabber-chat--handle-mam-sync-complete' marks it."
  (when (jabber-mam-sync--stale-p buffer)
    (with-current-buffer buffer
      (when (and (bound-and-true-p jabber-message-thread-id)
                 (get-buffer-window buffer t))
        (jabber-db-mark-message-thread-read
         (jabber-connection-bare-jid jabber-buffer-connection)
         jabber-message-thread-peer jabber-message-thread-type
         jabber-message-thread-id)
        ;; The read state is part of the parent's history (its thread
        ;; summary): a parent loaded before is stale now.  This buffer's
        ;; own refresh, below, comes after.
        (puthash (butlast (jabber-chat-buffer--history-key buffer)) (float-time)
                 jabber-mam-sync--changed)
        (jabber-mam-sync--queue-shown))
      (jabber-chat-buffer-refresh))))

(defun jabber-mam-sync--shown (frame)
  "Refresh the stale chat buffers FRAME's windows now show.
In `window-buffer-change-functions' (its global value, called with each
frame whose windows changed buffers): a buffer's history waits until a
window shows it."
  (when (and (> (hash-table-count jabber-mam-sync--changed) 0) (framep frame))
    (dolist (buffer (jabber-mam-sync--shown-stale (list frame)))
      (jabber-lifecycle--call-contained #'jabber-mam-sync--refresh buffer))))

(defun jabber-mam-sync--merge-next ()
  "Refresh one queued buffer once the user is idle and no input waits."
  (setq jabber-mam-sync--merge-timer nil)
  (let ((idle (current-idle-time)))
    (if (or (input-pending-p) (null idle)
            (< (float-time idle) jabber-mam-sync-merge-idle))
        (jabber-mam-sync--merge-arm)
      (let ((buffer (pop jabber-mam-sync--merge-queue)))
        (unless jabber-mam-sync--merge-queue (setq jabber-mam-sync--merge-tail nil))
        (remhash buffer jabber-mam-sync--merge-queued)
        (jabber-lifecycle--call-contained #'jabber-mam-sync--refresh buffer))
      (when jabber-mam-sync--merge-queue
        (jabber-mam-sync--merge-arm)))))

;;; Worker side

(define-error 'jabber-mam-sync-refused
  "The read-only sync session does not send this")

(defconst jabber-mam-sync--allowed-iq
  '(("set" (bind . "urn:ietf:params:xml:ns:xmpp-bind")
     (session . "urn:ietf:params:xml:ns:xmpp-session")
     (query . "urn:xmpp:mam:2"))
    ("get" (query . "http://jabber.org/protocol/disco#info")
     (ping . "urn:xmpp:ping"))
    ("result") ("error"))
  "IQs the worker's session sends: by type, the (ELEMENT . NAMESPACE) allowed.
Element and namespace both: MAM's preferences are a <prefs/> in the
archive namespace, and they are a server setting.  Replies (result,
error) to the server's requests pass whatever they hold.")

(defvar jabber-mam-sync--refused nil
  "In a worker: what its send gate refused, newest first.")

(defun jabber-mam-sync--allowed-p (sexp)
  "Non-nil when the worker's read-only session may send SEXP."
  (pcase (jabber-xml-node-name sexp)
    ((or 'auth 'response 'abort)
     (equal (jabber-xml-get-xmlns sexp) "urn:ietf:params:xml:ns:xmpp-sasl"))
    ('starttls (equal (jabber-xml-get-xmlns sexp) "urn:ietf:params:xml:ns:xmpp-tls"))
    ('iq (let* ((type (jabber-xml-get-attribute sexp 'type))
                (allowed (assoc type jabber-mam-sync--allowed-iq))
                (children (jabber-xml-node-children sexp))
                (child (car children)))
           (and allowed
                (or (null (cdr allowed))
                    (and (null (cdr children))
                         (member (cons (jabber-xml-node-name child)
                                       (jabber-xml-get-xmlns child))
                                 (cdr allowed)))))))))

(defun jabber-mam-sync--allowed-string-p (string)
  "Non-nil for a raw STRING the read-only session sends.
Its stream's opening tag (with the XML declaration), its close, and
whitespace; not Stream Management's elements, which it never enables."
  (or (string-match-p "\\`[ \t\r\n]*\\'" string)
      (equal string "</stream:stream>")
      (string-match-p "\\`\\(?:<\\?xml [^<>]*\\?>\\)?<stream:stream [^<>]*>[ \t\r\n]*\\'"
                      string)))

(defun jabber-mam-sync--gate (_jc sexp)
  "Signal `jabber-mam-sync-refused' unless the session may send SEXP.
`:before' advice on `jabber-send-sexp--raw': the caller sees a send
failure, never a success."
  (unless (jabber-mam-sync--allowed-p sexp)
    (push (jabber-xml-node-name sexp) jabber-mam-sync--refused)
    (signal 'jabber-mam-sync-refused (list (jabber-xml-node-name sexp)))))

(defun jabber-mam-sync--gate-string (_jc string)
  "Signal `jabber-mam-sync-refused' unless the session may send STRING.
`:before' advice on `jabber-send-string', the raw transport handoff."
  (unless (jabber-mam-sync--allowed-string-p string)
    (push (substring string 0 (min 40 (length string))) jabber-mam-sync--refused)
    (signal 'jabber-mam-sync-refused (list (substring string 0 (min 40 (length string)))))))

(defvar jabber-mam-sync--stored nil
  "In a worker: hash of (ACCOUNT PEER TYPE) to (INSERTED . CHANGED) counts
of committed results.")

(defun jabber-mam-sync--count-insert (_db account peer _resource _occupant-id
                                          _direction type &rest _)
  "Note a row inserted for ACCOUNT, PEER and TYPE (`:after' advice).
It counts once its result's transaction commits."
  (jabber-mam--note-effect account peer type t))

(defvar jabber-mam-sync--progress-at nil
  "In a worker: when the job last made progress.")

(defun jabber-mam-sync--note-progress (&rest _)
  "Note that the job made progress now (`:after' advice, hook)."
  (setq jabber-mam-sync--progress-at (float-time)))

(defvar jabber-mam-sync--pages nil
  "In a worker: hash of archive (room, peer or nil) to the pages it answered.")

(defvar jabber-mam-sync--timings nil
  "In a worker: (ARCHIVE-SPEC . SECONDS) each archive took to settle.")

(defun jabber-mam-sync--note-page (_jc _xml-data closure)
  "Count CLOSURE's page answered; the job made progress (`:after' advice)."
  (jabber-mam-sync--note-progress)
  (when-let* ((query (car-safe closure))
              ((consp query)))
    (let ((key (or (plist-get query :to) (plist-get query :with))))
      (puthash key (1+ (gethash key jabber-mam-sync--pages 0)) jabber-mam-sync--pages))))

(defun jabber-mam-sync--committed (effects)
  "Count a committed result's EFFECTS (`jabber-mam-committed-functions')."
  (jabber-mam-sync--note-progress)
  (dolist (effect effects)
    (let* ((key (list (nth 0 effect) (nth 1 effect) (nth 2 effect)))
           (counts (or (gethash key jabber-mam-sync--stored) (cons 0 0))))
      (puthash key (if (nth 3 effect)
                       (cons (1+ (car counts)) (cdr counts))
                     (cons (car counts) (1+ (cdr counts))))
               jabber-mam-sync--stored))))

(defvar jabber-mam-sync--failure nil
  "In a worker: the reason the query that last failed gave.
`jabber-mam-query-failed-functions' runs before the query's callbacks,
so an archive's outcome takes it from here.")

(defun jabber-mam-sync--note-failure (_query reason)
  "Keep REASON for the failed query's outcome.
A function of `jabber-mam-query-failed-functions'."
  (setq jabber-mam-sync--failure reason))

(defvar jabber-mam-sync--stalls nil
  "In a worker: the watchdog's actions, as (ARCHIVE ACTION FOUND), newest first.")

(defun jabber-mam-sync--note-stall (query action found)
  "Record the watchdog's ACTION on QUERY and what it FOUND.
A function of `jabber-mam-stall-functions'."
  (jabber-mam-sync--note-progress)
  (push (list (jabber-mam-archive-name query) action found) jabber-mam-sync--stalls))

(defun jabber-mam-sync--cpu-seconds ()
  "This process's user and system CPU seconds, or nil."
  (ignore-errors
    (with-temp-buffer
      (insert-file-contents "/proc/self/stat")
      (let ((fields (split-string (buffer-substring (1+ (search-forward ")")) (point-max)))))
        (/ (+ (string-to-number (nth 11 fields)) (string-to-number (nth 12 fields)))
           100.0)))))

(defun jabber-mam-sync--condition (result)
  "The error condition a disco RESULT names, as a string, or \"none\"."
  (or (ignore-errors
        (symbol-name (jabber-xml-node-name (car (jabber-xml-node-children result)))))
      "none"))

(defun jabber-mam-sync--worker-start (jc archive done)
  "Catch up ARCHIVE on JC once disco says it has MAM; DONE gets the outcome."
  (let* ((room (plist-get archive :room))
         (with (plist-get archive :with))
         (target (or room (jabber-connection-bare-jid jc)))
         (finish (lambda (outcome)
                   (funcall done (if (eq outcome 'failed)
                                     (cons 'failed (or jabber-mam-sync--failure "the query failed"))
                                   outcome)))))
    (condition-case err
        (jabber-disco-get-info
         jc target nil
         (lambda (jc _closure result)
           (cond
            ((or (not (listp result)) (eq (car result) 'error))
             ;; The entity did not answer: that is a failure; one that
             ;; answers without MAM has no archive to read.
             (funcall done (cons 'failed (format "no disco answer (%s)"
                                                 (jabber-mam-sync--condition result)))))
            ((member jabber-mam-xmlns (cadr result))
               (condition-case err
                   (cond (room (jabber-mam--muc-catch-up jc room finish))
                         (with
                          (let ((progress (jabber-db-mam-progress target target with)))
                            (jabber-mam--query jc (car progress) nil with (cadr progress)
                                               nil nil nil nil finish)))
                         (t (jabber-mam-sync--sliced-catch-up jc finish)))
                 (error (funcall done (cons 'failed (error-message-string err))))))
            (t (funcall done 'unsupported))))
         nil)
      (error (funcall done (cons 'failed (error-message-string err)))))))

(defun jabber-mam-sync--stamp (seconds)
  "SECONDS since the epoch as an XEP-0082 time."
  (format-time-string "%Y-%m-%dT%H:%M:%SZ" seconds t))

(defun jabber-mam-sync--slice-from (slice)
  "Where SLICE's remaining time starts: its newest message, else its start."
  (max (plist-get slice :start)
       (or (plist-get (plist-get slice :query) :latest) 0)))

(defun jabber-mam-sync--slice-span (slice now)
  "Seconds of archive SLICE has left at NOW."
  (- (or (plist-get slice :end) now) (jabber-mam-sync--slice-from slice)))

(defun jabber-mam-sync--widest (slices now)
  "The running slice of SLICES with the most time left at NOW, if it can be
split: each half keeps `jabber-mam-sync-min-slice' seconds."
  (let ((widest nil))
    (dolist (slice slices)
      (when (and (memq (plist-get slice :query) jabber-mam--syncing)
                 (> (jabber-mam-sync--slice-span slice now) (* 2 jabber-mam-sync-min-slice))
                 (or (null widest)
                     (> (jabber-mam-sync--slice-span slice now) (jabber-mam-sync--slice-span widest now))))
        (setq widest slice)))
    widest))

(defun jabber-mam-sync--sliced-catch-up (jc done)
  "Catch up JC's personal archive in time slices paged at once.
DONE gets `success' or (failed . REASON).  The first slice pages forward
from the stored cursor and moves it, as `jabber-mam--catch-up' does.
Once its first page shows more to come, the widest slice is halved until
`jabber-mam-sync-slices' run; whenever one finishes, the widest left
gives it its later half.  A narrowed slice's next page stops at its new
end; what a page in flight brings past it is stored once anyway.  When
every slice has succeeded the cursor moves to the newest message; should
one fail, it stays where the first slice left it, and the next catch-up
reads the rest again."
  (let* ((account (jabber-connection-bare-jid jc))
         (state (list :slices nil :running 0 :failure nil :split nil :first nil))
         (start-slice nil) (split nil) (slice-done nil))
    (setq split
          (lambda ()
            (let* ((now (float-time))
                   (victim (jabber-mam-sync--widest (plist-get state :slices) now)))
              (when victim
                (let* ((from (jabber-mam-sync--slice-from victim))
                       (end (plist-get victim :end))
                       (mid (+ from (/ (- (or end now) from) 2.0))))
                  (plist-put victim :end mid)
                  (plist-put (plist-get victim :query) :end (jabber-mam-sync--stamp mid))
                  (funcall start-slice mid end)
                  t)))))
    (setq slice-done
          (lambda (outcome)
            (plist-put state :running (1- (plist-get state :running)))
            (when (and (not (eq outcome 'success)) (not (plist-get state :failure)))
              ;; The failure hook ran just before this callback.
              (plist-put state :failure (if (eq outcome 'failed)
                                            (or jabber-mam-sync--failure "a slice failed")
                                          "cancelled")))
            (unless (plist-get state :failure)
              (while (and (< (plist-get state :running) (max 1 jabber-mam-sync-slices))
                          (funcall split))))
            (when (zerop (plist-get state :running))
              (funcall done (jabber-mam-sync--slices-settled account state)))))
    (setq start-slice
          (lambda (start end)
            (let ((slice (list :start start :end end :query nil)))
              (plist-put state :running (1+ (plist-get state :running)))
              (plist-put state :slices
                         (sort (cons slice (plist-get state :slices))
                               (lambda (a b) (< (plist-get a :start) (plist-get b :start)))))
              (plist-put slice :query
                         (jabber-mam--query
                          jc nil nil nil (jabber-mam-sync--stamp start) nil nil nil nil
                          slice-done nil
                          (list :end (and end (jabber-mam-sync--stamp end)) :forward nil))))))
    ;; The first slice: forward from the cursor, open at the end.
    (let* ((progress (jabber-db-mam-progress account account nil))
           (slice (list :start 0 :end nil :query nil)))
      (plist-put state :running 1)
      (plist-put state :slices (list slice))
      (plist-put state :first slice)
      (plist-put slice :query
                 (jabber-mam--query
                  jc (car progress) nil nil (cadr progress) nil nil nil nil
                  slice-done nil
                  (list :on-page
                        (lambda (query)
                          (unless (plist-get state :split)
                            (plist-put state :split t)
                            (plist-put query :on-page nil)
                            (while (and (< (plist-get state :running)
                                           (max 1 jabber-mam-sync-slices))
                                        (funcall split))))))))
      nil)))

(defun jabber-mam-sync--slices-settled (account state)
  "The outcome of ACCOUNT's sliced catch-up STATE, its cursor moved on success.
The cursor moves to the newest UID a slice reached, the latest slice's
first, with the first slice's coverage: the slices cover it all."
  (if-let* ((failure (plist-get state :failure)))
      (cons 'failed failure)
    (let ((slices (plist-get state :slices)))
      (when (cdr slices)
        (let ((newest (seq-some (lambda (slice) (plist-get (plist-get slice :query) :last-uid))
                                (reverse slices)))
              (first (plist-get (plist-get state :first) :query)))
          (when-let* ((newest newest)
                      (db (jabber-db-ensure-open)))
            (jabber-db--advance-mam-progress
             db account account nil
             (car (jabber-db-mam-progress account account nil))
             newest (plist-get first :coverage-start)))))
      'success)))

(defun jabber-mam-sync-worker-run (account archives)
  "In a worker VM: catch up ARCHIVES of ACCOUNT over a session of its own.
ACCOUNT is `jabber-mam-sync--account''s plist, ARCHIVES the specs of
`jabber-mam-sync--archives'.  The session sends no presence, joins no
room, never enables Stream Management, and passes every stanza and raw
string through the send gate.  Return the summary: (:archives
\((ROOM-OR-PEER-OR-NIL TYPE OUTCOME STORED REASON)...) :peers ((ACCOUNT
PEER TYPE STORED CHANGED)...) :refused WHAT :seconds S :pid PID :cpu S), with
:error REASON when the session never came up."
  (let* ((inhibit-message t)
         (message-log-max nil)
         (start (float-time))
         (jabber-post-connect-hooks nil)
         (jabber-post-resume-hooks nil)
         (jabber-lifecycle-session-bootstrap-functions nil)
         (jabber-sm-enable nil)
         (jabber-auto-reconnect nil)
         (jabber-debug-log-xml nil)
         (jabber-chat--crypto-loaded t)
         (jabber-mam-sync-complete-functions nil)
         (jabber-mam-peer-syncing-functions nil)
         (jabber-mam-committed-functions (list #'jabber-mam-sync--committed))
         (jabber-mam-enable t)
         (jabber-mam-background nil)
         ;; Each page is requested from the previous one's answer, with no
         ;; timer between them; the loop below runs the watchdog too.
         (jabber-mam-page-delay 0)
         (jabber-mam-catch-up-min-page-size
          (max jabber-mam-catch-up-min-page-size jabber-mam-sync-page-size))
         ;; fsm.el logs every event to a buffer by default.
         (fsm-debug nil)
         (jabber-mam-query-failed-functions (list #'jabber-mam-sync--note-failure))
         (jabber-mam-stall-functions (list #'jabber-mam-sync--note-stall))
         (jabber-mam-sync--failure nil)
         (jabber-mam-sync--stalls nil)
         (jabber-mam--batch (jabber-mam-make-batch))
         (jabber-mam-sync--pages (make-hash-table :test #'equal))
         (jabber-mam-sync--timings nil)
         (jabber-db-busy-timeout (max jabber-db-busy-timeout 30000))
         (jabber-mam--room-nicks
          (let ((nicks (make-hash-table :test #'equal)))
            (dolist (a archives nicks)
              (when (and (plist-get a :room) (plist-get a :nick))
                (puthash (plist-get a :room) (plist-get a :nick) nicks)))))
         (jabber-mam-sync--refused nil)
         (jabber-mam-sync--stored (make-hash-table :test #'equal))
         (jabber-mam-sync--progress-at start)
         (queue (copy-sequence archives))
         (total (length archives))
         (running 0)
         (outcomes nil)
         (settled-count 0)
         (connected nil)
         jc next)
    (setq next
          (lambda ()
            (while (and queue (< running (max 1 jabber-mam-sync-parallel)))
              (let ((archive (pop queue)) (settled nil) (began (float-time)))
                (setq running (1+ running))
                (jabber-mam-sync--worker-start
                 jc archive
                 (lambda (outcome)
                   (unless settled
                     (setq settled t running (1- running)
                           settled-count (1+ settled-count))
                     (push (cons archive outcome) outcomes)
                     (push (cons archive (- (float-time) began)) jabber-mam-sync--timings)
                     (funcall next))))))))
    (advice-add 'jabber-send-sexp--raw :before #'jabber-mam-sync--gate)
    (advice-add 'jabber-send-string :before #'jabber-mam-sync--gate-string)
    (advice-add 'jabber-db--insert-message :after #'jabber-mam-sync--count-insert)
    (advice-add 'jabber-mam--handle-fin :after #'jabber-mam-sync--note-page)
    (jabber-db-share-writes 'defer)
    (unwind-protect
        (progn
          ;; The session's bootstrap, not `jabber-post-connect-hooks': those
          ;; run after the initial roster, which this session never asks for.
          (setq jabber-lifecycle-session-bootstrap-functions
                (list (lambda (c) (when (eq c jc) (setq connected t) (funcall next)))))
          (setq jc (car (jabber-connect (plist-get account :username)
                                        (plist-get account :server)
                                        (plist-get account :resource) nil
                                        (plist-get account :password)
                                        (plist-get account :network-server)
                                        (plist-get account :port)
                                        (plist-get account :connection-type)
                                        (plist-get account :proxy))))
          (while (and (< settled-count total)
                      (< (- (float-time) jabber-mam-sync--progress-at)
                         jabber-mam-sync-timeout)
                      (memq jc jabber-connections)
                      (or connected (< (- (float-time) start) 60)))
            (accept-process-output nil 0.1)
            ;; Its timer commits it first; this is the backstop.
            (jabber-mam-batch-commit)
            (jabber-mam-check-stalls))
          (jabber-mam-sync--summary
           account archives outcomes connected (- (float-time) start)
           (if (memq jc jabber-connections)
               (format "no progress for %ds" jabber-mam-sync-timeout)
             "the sync session ended")))
      (ignore-errors (jabber-mam-batch-commit))
      (dolist (c jabber-connections)
        (ignore-errors (jabber-disconnect-one c)))
      (advice-remove 'jabber-send-sexp--raw #'jabber-mam-sync--gate)
      (advice-remove 'jabber-send-string #'jabber-mam-sync--gate-string)
      (advice-remove 'jabber-db--insert-message #'jabber-mam-sync--count-insert)
      (advice-remove 'jabber-mam--handle-fin #'jabber-mam-sync--note-page)
      (jabber-db-share-writes nil))))

(defun jabber-mam-sync--summary (account archives outcomes connected seconds
                                         &optional unsettled)
  "The worker's summary of ARCHIVES' OUTCOMES for ACCOUNT; see the runner.
Counts are of committed results only.  An archive with no outcome is
`incomplete', for the reason UNSETTLED."
  (let* ((rooms (make-hash-table :test #'equal))
         (by-peer (make-hash-table :test #'equal))
         (settled (make-hash-table :test #'eq))
         (personal 0)
         (peers nil))
    (dolist (a archives)
      (when (plist-get a :room) (puthash (plist-get a :room) t rooms)))
    ;; The first outcome and error recorded for each archive.
    (dolist (outcome (reverse outcomes))
      (unless (gethash (car outcome) settled) (puthash (car outcome) (cdr outcome) settled)))
    (maphash (lambda (key counts)
               (push (append key (list (car counts) (cdr counts))) peers)
               (puthash (nth 1 key) (+ (car counts) (gethash (nth 1 key) by-peer 0)) by-peer)
               (unless (or (gethash (nth 1 key) rooms) (equal (nth 2 key) "groupchat"))
                 (setq personal (+ personal (car counts)))))
             jabber-mam-sync--stored)
    (append
     (unless connected (list :error "the sync session did not connect"))
     (list
      :archives
      (mapcar
       (lambda (archive)
         (let* ((room (plist-get archive :room))
                (with (plist-get archive :with))
                (outcome (gethash archive settled 'incomplete))
                (reason (cond ((consp outcome) (cdr outcome))
                              ((eq outcome 'incomplete) unsettled)))
                (outcome (if (consp outcome) (car outcome) outcome)))
           (list (or room with) (if room "groupchat" "chat") outcome
                 (cond (room (gethash room by-peer 0))
                       (with (gethash with by-peer 0))
                       (t personal))
                 reason)))
       archives)
      :peers peers
      :account (concat (plist-get account :username) "@" (plist-get account :server))
      :refused (reverse jabber-mam-sync--refused)
      :stalls (reverse jabber-mam-sync--stalls)
      ;; (ARCHIVE PAGES SECONDS), ARCHIVE as in :archives.
      :timing (mapcar (lambda (archive)
                        (let ((key (or (plist-get archive :room) (plist-get archive :with))))
                          (list key
                                (if jabber-mam-sync--pages (gethash key jabber-mam-sync--pages 0) 0)
                                (cdr (assq archive jabber-mam-sync--timings)))))
                      archives)
      :seconds seconds
      :pid (emacs-pid)
      :cpu (jabber-mam-sync--cpu-seconds)))))

;;; Registration

(add-hook 'window-buffer-change-functions #'jabber-mam-sync--shown)
(add-hook 'jabber-pre-disconnect-hook #'jabber-mam-sync--cancel-all)
(add-hook 'jabber-lifecycle-session-reset-functions #'jabber-mam-sync--cancel)
(add-hook 'jabber-lost-connection-hooks #'jabber-mam-sync--cancel)

(provide 'jabber-mam-sync)
;;; jabber-mam-sync.el ends here
