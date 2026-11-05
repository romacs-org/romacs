;;; jabber-mam.el --- XEP-0313 Message Archive Management  -*- lexical-binding: t; -*-

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

;; XEP-0313 (Message Archive Management) support.
;; Queries the server's message archive on connect to sync missed
;; messages across devices.  Results are stored in the local database
;; and displayed in open chat buffers.
;;
;; Pagination uses XEP-0059 (Result Set Management).
;; Deduplication uses XEP-0359 stanza-id / server-id.

;;; Code:

(require 'jabber-xml)
(require 'jabber-util)
(require 'jabber-db)
(require 'jabber-chat)
(require 'jabber-disco)
(require 'jabber-iq)
(require 'jabber-lifecycle)
(require 'jabber-message-correct)
(require 'jabber-muc-state)

(eval-when-compile (require 'cl-lib))

(defvar jabber-buffer-connection)       ; jabber-chatbuffer.el
(defvar jabber-chatting-with)           ; jabber-chat.el
(defvar jabber-group)                   ; jabber-muc.el

;;; Constants

(defconst jabber-mam-xmlns "urn:xmpp:mam:2"
  "Namespace for XEP-0313 MAM.")

(defconst jabber-mam-rsm-xmlns "http://jabber.org/protocol/rsm"
  "Namespace for XEP-0059 Result Set Management.")

(defconst jabber-mam-forward-xmlns "urn:xmpp:forward:0"
  "Namespace for XEP-0297 Stanza Forwarding.")

(defconst jabber-mam-delay-xmlns "urn:xmpp:delay"
  "Namespace for XEP-0203 Delayed Delivery.")

;;; Customization

(defcustom jabber-mam-enable t
  "Whether to sync messages via MAM on connect."
  :type 'boolean
  :group 'jabber)

(defcustom jabber-mam-page-size 50
  "Number of messages to request per MAM page."
  :type 'integer
  :group 'jabber)

(defcustom jabber-mam-catch-up-min-page-size 50
  "Fewest results a catch-up page asks for, whatever `jabber-mam-page-size' is.
A smaller page only adds round trips; servers clamp a page to their own
maximum.  `jabber-mam-sync-buffer' still asks for its own count."
  :type 'natnum
  :group 'jabber)

(defcustom jabber-mam-room-history 200
  "Messages a room's catch-up fetches when the room has no cursor yet.
Instead of the room's whole archive, which for a busy public room is
years: the newest of them becomes the room's cursor and the oldest the
start of its local coverage.  A room with a cursor catches up forward
from it, completely."
  :type 'natnum
  :group 'jabber)

(defcustom jabber-mam-catch-up-days 3
  "Legacy initial catch-up window, retained for configuration compatibility.
Automatic catch-up now scans unbounded when durable coverage is unknown;
using a recent window after migration could silently omit older messages."
  :type '(choice integer (const :tag "Fetch all" nil))
  :group 'jabber)

;;; Hooks

(defvar jabber-mam-peer-syncing-functions nil
  "Hook run when a peer's MAM sync state changes.
Each function receives three arguments: PEER (bare JID), TYPE
\(\"groupchat\" or \"chat\"), and SYNCING-P (non-nil when sync
starts, nil when it ends).")

(defvar jabber-mam-sync-complete-functions nil
  "Hook run after MAM stores messages for one or more peers.
Each function receives one argument: a list of (ACCOUNT PEER TYPE) entries.")

(defvar jabber-mam-query-failed-functions (list #'jabber-mam--report-failure)
  "Abnormal hook run once when a query fails, with QUERY and its REASON.
REASON is a string naming what went wrong; see `jabber-mam-archive-name'
for the archive.  A cancelled query is no failure.")

(defvar jabber-mam-stall-functions (list #'jabber-mam--log-stall)
  "Abnormal hook run when the watchdog acts on a stalled QUERY.
Each function receives QUERY, the ACTION taken (`request': the next page
was overdue and never requested; `retry': the page went unanswered and
is asked for again) and a plist of what the watchdog found.")

;;; Paging and its watchdog

(defvar jabber-mam-page-delay 0.1
  "Seconds between a page's answer and the request for the next, or 0.
At 0 the next page is requested from the answer itself; otherwise from a
timer, which lets a session's input run between pages.")

(defcustom jabber-mam-page-timeout 30
  "Seconds a page may go unanswered before it is asked for again.
A page silent that long twice fails its query, naming the archive.
Results arriving count as an answer."
  :type 'number
  :group 'jabber)

(defvar jabber-mam-stall-grace 2
  "Seconds past its due time before the watchdog requests a next page itself.")

(defvar jabber-mam--watchdog nil
  "The one-shot timer of `jabber-mam-check-stalls' at the next deadline, or nil.
It fires only when a query reaches a deadline: in steady paging, never.")

(defvar jabber-mam--checked-at 0
  "When incoming input last ran `jabber-mam-check-stalls'.")

;;; Internal state

(defvar jabber-mam--syncing nil
  "Active query plists, including queries waiting for their next page.
Each owns its connection/session predicate, archive target, page token,
pagination timer and completion callbacks.")

(defvar jabber-mam-committed-functions nil
  "Abnormal hook run after an archived result's transaction commits.
Each function receives the result's effects, a list of entries
\(ACCOUNT PEER TYPE INSERTED): one for each peer whose history the result
changed, and one with INSERTED t for each message row it added (see
`jabber-mam--note-effect').  A result rolled back has none.")

(defvar jabber-mam--batch nil
  "The batch archived results commit in, or nil for a transaction each.
A worker binds it to `jabber-mam-make-batch'.  The first result opens
the batch's transaction; its owner commits it (`jabber-mam-batch-commit')
before its process waits for input, as `jabber-mam-sync-worker-run'
does, so a batch holds what arrives while the stream keeps coming.  It
commits earlier once it holds `jabber-mam-batch-size' results, once it
has been open `jabber-mam-batch-age' seconds, as soon as the live
session wants to write (`jabber-db-share-writes'), and before a query
reports success.
Collections wait while it is open: one would hold the write lock the
session may be waiting for as long as it takes.")

(defvar jabber-mam--effects nil
  "While a result's transaction runs with `jabber-mam-committed-functions':
a cons whose cdr collects the transaction's effects.")

(defun jabber-mam--note-effect (account peer type &optional inserted)
  "Record that the current result changed PEER's history on ACCOUNT.
TYPE is the message type; INSERTED non-nil when it added a row."
  (when jabber-mam--effects
    (push (list account peer type inserted) (cdr jabber-mam--effects))))

(defvar jabber-mam--dirty-peers nil
  "Peers awaiting refresh after their results, as (ACCOUNT PEER TYPE).")

(defvar jabber-mam--redraw-peers nil
  "Committed peers awaiting a progressive redraw, as (ACCOUNT PEER TYPE).")

(defvar jabber-mam--redraw-delay 1.0
  "Seconds to coalesce committed pages before redrawing their buffers.")

(defvar jabber-mam--redraw-timer nil
  "Pending timer for `jabber-mam--redraw-committed', or nil.")

(defvar jabber-mam--room-nicks nil
  "Hash of ROOM to our nickname there, for rooms read without being joined.
The background sync's worker reads room archives from a session that
joined none; the live session tells it our nicknames.  Nil: none.")

(defvar jabber-mam--peer-syncing nil
  "Alist of active automatic peer catch-ups.
Each entry has the shape ((JC PEER) . TOKEN).")

;;; Public predicates

(defun jabber-mam-syncing-p ()
  "Return non-nil if any MAM sync is in progress."
  (not (null jabber-mam--syncing)))

;;; Query building

(defvar jabber-mam--queryid-counter 0
  "Monotonic counter for unique MAM query IDs.")

(defun jabber-mam--make-queryid ()
  "Generate a unique query ID for MAM."
  (format "mam-%d-%d"
          (cl-incf jabber-mam--queryid-counter)
          (floor (float-time))))

(defun jabber-mam--begin-peer-sync (jc peer)
  "Record an automatic MAM catch-up for JC and PEER.
Return its unique token, or nil when one was already active."
  (let ((key (list jc peer)))
    (unless (assoc key jabber-mam--peer-syncing #'equal)
      (let ((token (make-symbol "jabber-mam-peer-sync-")))
        (push (cons key token) jabber-mam--peer-syncing)
        token))))

(defun jabber-mam--peer-sync-active-p (jc peer token)
  "Return non-nil when TOKEN owns JC and PEER's catch-up."
  (eq token
      (cdr (assoc (list jc peer) jabber-mam--peer-syncing #'equal))))

(defun jabber-mam--end-peer-sync (jc peer token)
  "Forget TOKEN's automatic MAM catch-up for JC and PEER."
  (when-let* ((entry (assoc (list jc peer)
                            jabber-mam--peer-syncing #'equal))
              ((eq token (cdr entry))))
    (setq jabber-mam--peer-syncing
          (delq entry jabber-mam--peer-syncing))
    t))

(defun jabber-mam--signal-peer-syncing (jc peer type syncing-p)
  "Show whether JC is syncing PEER of TYPE, then run the public hook.
SYNCING-P is non-nil when sync starts, nil when it ends.  The built-in
indicator needs JC to find an account's room buffer, so it is updated
here rather than from `jabber-mam-peer-syncing-functions'.  Contain
failures so a faulty handler cannot strand a catch-up."
  (jabber-lifecycle--call-contained
   #'jabber-chat--update-mam-syncing jc peer type syncing-p)
  (jabber-lifecycle--dispatch-contained
   'jabber-mam-peer-syncing-functions peer type syncing-p))

(defun jabber-mam--finish-peer-sync (jc peer token)
  "Finish TOKEN's peer catch-up for JC and PEER."
  (when (jabber-mam--end-peer-sync jc peer token)
    (jabber-mam--signal-peer-syncing jc peer "chat" nil)))

(defun jabber-mam--release-page (query)
  "Release QUERY's page at most once."
  (plist-put query :page nil)
  (when-let* ((id (plist-get query :iq-id)))
    (setq jabber-open-info-queries
          (delete (assoc id jabber-open-info-queries) jabber-open-info-queries))
    (plist-put query :iq-id nil)))

(defun jabber-mam--complete-query (query outcome)
  "Retire QUERY with OUTCOME before releasing resources and calling consumers.
OUTCOME is `success', `failed' or `cancelled'; see `jabber-mam--query'.
Contain callback errors and quits so other queries can finish cleanup."
  (when (memq query jabber-mam--syncing)
    (unless (eq outcome 'success)
      (plist-put query :failed t))
    (setq jabber-mam--syncing (delq query jabber-mam--syncing))
    (when-let* ((timer (plist-get query :timer)))
      (cancel-timer timer)
      (plist-put query :timer nil))
    (unless jabber-mam--syncing
      (jabber-mam--unwatch))
    (unwind-protect
        (condition-case err
            (jabber-mam--release-page query)
          ((error quit)
           (setq outcome 'failed)
           (signal (car err) (cdr err))))
      (when (eq outcome 'failed)
        (jabber-lifecycle--dispatch-contained
         'jabber-mam-query-failed-functions query
         (or (plist-get query :error) "the query failed")))
      (when-let* ((callback (plist-get query :callback)))
        (jabber-lifecycle--call-contained callback))
      (when-let* ((callback (plist-get query :result-callback)))
        (jabber-lifecycle--call-contained callback outcome)))
    (jabber-mam--redraw-dirty)))

(defun jabber-mam--fail-query (query reason)
  "Fail QUERY for REASON, a string the failure hook reports."
  (plist-put query :error reason)
  (jabber-mam--complete-query query 'failed))

(defun jabber-mam-archive-name (query)
  "A name for QUERY's archive: the room, the peer, or the personal archive."
  (or (plist-get query :to)
      (and (plist-get query :with) (format "chat with %s" (plist-get query :with)))
      "personal archive"))

(defun jabber-mam--report-failure (query reason)
  "Say that QUERY failed for REASON, naming its archive."
  (message "Jabber: sync failed for %s: %s" (jabber-mam-archive-name query) reason))

(defun jabber-mam--log-stall (query action found)
  "Log the watchdog's ACTION on QUERY and what it FOUND, without echoing."
  (let ((inhibit-message t))
    (message "Jabber: %s: %s %S" (jabber-mam-archive-name query)
             (if (eq action 'request) "next page was never requested; requested it"
               "page went unanswered; asked again")
             found)))

;;; The watchdog

(defun jabber-mam--stall-action (query now)
  "What the watchdog does about QUERY at NOW: nil, `request', `retry' or `fail'.
`request': no page is in flight and the next one is overdue by more than
`jabber-mam-stall-grace'.  `retry': the page in flight has gone unanswered
for `jabber-mam-page-timeout' seconds; `fail': it has after a retry."
  (if (not (plist-get query :page))
      (let ((due (plist-get query :due-at)))
        (and due (> now (+ due jabber-mam-stall-grace)) 'request))
    (let ((heard (plist-get query :heard-at)))
      (and heard (> (- now heard) jabber-mam-page-timeout)
           (if (plist-get query :silent) 'fail 'retry)))))

(defun jabber-mam--timer-state (timer)
  "What the watchdog finds of a page TIMER: whether it is one and queued."
  (list :timer (timerp timer)
        :queued (and (timerp timer) (memq timer timer-list) t)
        :triggered (and (timerp timer) (timer--triggered timer))))

(defun jabber-mam-check-stalls (&optional now)
  "Act on each registered query that stalled, as `jabber-mam--stall-action' says.
A next page never requested is requested; a silent page is asked for once
more, then its query fails.  NOW defaults to the current time.  Called by
the watchdog timer at the next deadline, by incoming stanzas at most
once a second (`jabber-mam--input-check'), and by a worker's own loop:
a stall never waits on one timer alone."
  (let ((now (or now (float-time))))
    (dolist (query (copy-sequence jabber-mam--syncing))
      (when (memq query jabber-mam--syncing)
        (pcase (jabber-mam--stall-action query now)
          ('request
           (let ((found (jabber-mam--timer-state (plist-get query :timer))))
             (when (timerp (plist-get query :timer))
               (cancel-timer (plist-get query :timer)))
             (plist-put query :timer nil)
             (jabber-lifecycle--dispatch-contained
              'jabber-mam-stall-functions query 'request found)
             (jabber-mam--send-page query)))
          ('retry
           (jabber-mam--release-page query)
           (plist-put query :silent t)
           ;; The new attempt gets the page again: what the abandoned one
           ;; received leaves the count, and its late results are told
           ;; apart by their queryid (`jabber-mam--process-message').
           (plist-put query :total (- (or (plist-get query :total) 0)
                                      (or (plist-get query :received) 0)))
           (plist-put query :retired-ids (cons (plist-get query :id)
                                               (plist-get query :retired-ids)))
           (plist-put query :id (jabber-mam--make-queryid))
           (jabber-lifecycle--dispatch-contained
            'jabber-mam-stall-functions query 'retry
            (list :silent (- now (plist-get query :heard-at))))
           (jabber-mam--send-page query))
          ('fail
           (jabber-mam--fail-query
            query (format "no answer for %ss" jabber-mam-page-timeout))))))
    (jabber-mam--watch)))

(defun jabber-mam--deadline (query)
  "When the watchdog must next look at QUERY, or nil.
A page in flight: `jabber-mam-page-timeout' after it was last heard from.
Between pages: `jabber-mam-stall-grace' after the next is due."
  (if (plist-get query :page)
      (when-let* ((heard (plist-get query :heard-at)))
        (+ heard jabber-mam-page-timeout))
    (when-let* ((due (plist-get query :due-at)))
      (+ due jabber-mam-stall-grace))))

(defun jabber-mam--watch ()
  "Arm the watchdog for the registered queries' earliest deadline.
An armed watchdog due no later stays; a lost one is armed again.  With
no query registered, stop it."
  (let ((next (seq-reduce (lambda (earliest query)
                            (let ((deadline (jabber-mam--deadline query)))
                              (if (and deadline (or (null earliest) (< deadline earliest)))
                                  deadline earliest)))
                          jabber-mam--syncing nil))
        (armed jabber-mam--watchdog))
    (cond
     ((null jabber-mam--syncing) (jabber-mam--unwatch))
     ((null next))
     ((and (timerp armed) (memq armed timer-list)
           (<= (float-time (timer--time armed)) next)))
     (t
      (jabber-mam--unwatch)
      ;; A little past the deadline, so the check finds it passed.
      (setq jabber-mam--watchdog
            (run-at-time (max 0.01 (+ 0.05 (- next (float-time)))) nil
                         #'jabber-mam-check-stalls))))))

(defun jabber-mam--input-check (_jc _xml-data)
  "Check for stalls as input arrives, at most once a second (a chain handler).
Presence, messages and IQs keep arriving on a live session, so a stall
is found even if every timer were lost."
  (when jabber-mam--syncing
    (let ((now (float-time)))
      (when (> (- now jabber-mam--checked-at) 1)
        (setq jabber-mam--checked-at now)
        (jabber-mam-check-stalls now)))))

(defun jabber-mam--unwatch ()
  "Stop the watchdog."
  (when (timerp jabber-mam--watchdog)
    (cancel-timer jabber-mam--watchdog))
  (setq jabber-mam--watchdog nil))

(defun jabber-mam--build-query (queryid &optional with start after-id max
                                        before-id end)
  "Build a MAM <query> sexp.
QUERYID is echoed in results for correlation.
WITH filters by JID, START and END are XEP-0082 datetime strings.
AFTER-ID is an RSM cursor for forward pagination.
MAX is the page size.
BEFORE-ID is an RSM cursor for backward pagination; when t, emit
an empty <before/> element (meaning \"last page\")."
  (let ((form-fields nil)
        (rsm-children nil))
    ;; Data form fields
    (when (or with start end)
      (push `(field ((var . "FORM_TYPE") (type . "hidden"))
                    (value () ,jabber-mam-xmlns))
            form-fields)
      (when with
        (push `(field ((var . "with"))
                      (value () ,with))
              form-fields))
      (when start
        (push `(field ((var . "start"))
                      (value () ,start))
              form-fields))
      (when end
        (push `(field ((var . "end"))
                      (value () ,end))
              form-fields))
      (setq form-fields (nreverse form-fields)))
    ;; RSM
    (when max
      (push `(max () ,(number-to-string max)) rsm-children))
    (when after-id
      (push `(after () ,after-id) rsm-children))
    (when before-id
      (if (eq before-id t)
          (push '(before ()) rsm-children)
        (push `(before () ,before-id) rsm-children)))
    (setq rsm-children (nreverse rsm-children))
    ;; Build query
    `(query ((xmlns . ,jabber-mam-xmlns)
             (queryid . ,queryid))
            ,@(when form-fields
                (list `(x ((xmlns . "jabber:x:data")
                           (type . "submit"))
                          ,@form-fields)))
            ,@(when rsm-children
                (list `(set ((xmlns . ,jabber-mam-rsm-xmlns))
                            ,@rsm-children))))))

;;; Result parsing

(defun jabber-mam--parse-result (xml-data)
  "Extract MAM result from the <message> stanza XML-DATA.
Return (ARCHIVE-ID DELAY-STAMP INNER-MESSAGE) or nil."
  (when-let* ((result-el (jabber-xml-child-with-xmlns
                          xml-data jabber-mam-xmlns)))
    (let* ((archive-id (jabber-xml-get-attribute result-el 'id))
           (fwd-el (car (jabber-xml-get-children result-el 'forwarded)))
           (delay-el (and fwd-el
                          (car (jabber-xml-get-children fwd-el 'delay))))
           (stamp (and delay-el
                       (jabber-xml-get-attribute delay-el 'stamp)))
           (inner-msg (and fwd-el
                           (car (jabber-xml-get-children fwd-el 'message)))))
      (when inner-msg
        (list archive-id stamp inner-msg)))))

(defun jabber-mam--parse-fin (xml-data)
  "Return validated MAM fin metadata from XML-DATA, or nil."
  (let* ((fins (seq-filter
                (lambda (el) (and (eq (jabber-xml-node-name el) 'fin)
                                  (equal (jabber-xml-get-xmlns el) jabber-mam-xmlns)))
                (jabber-xml-node-children xml-data)))
         (fin (car fins))
         (complete (jabber-xml-get-attribute fin 'complete))
         (sets (jabber-xml-get-children fin 'set))
         (set (car sets))
         (lasts (jabber-xml-get-children set 'last))
         (firsts (jabber-xml-get-children set 'first))
         (last (car (jabber-xml-node-children (car lasts))))
         (first (car (jabber-xml-node-children (car firsts)))))
    (when (and (= (length fins) 1)
               (member complete '(nil "true" "false" "1" "0"))
               (<= (length sets) 1)
               (or (null set) (equal (jabber-xml-get-xmlns set) jabber-mam-rsm-xmlns))
               (<= (length lasts) 1) (<= (length firsts) 1)
               (or (null lasts) (and (stringp last) (not (string-empty-p last))
                                            (= (length (cddar lasts)) 1)))
               (or (null firsts) (and (stringp first) (not (string-empty-p first)))))
      (list :complete (not (null (member complete '("true" "1"))))
            :first first :last last))))

;;; Message chain handler

(defun jabber-mam--unwrap-into (outer inner &optional archive-id)
  "Replace OUTER stanza's attributes and children with INNER's.
Marks the stanza as MAM-origin so downstream handlers can suppress
outgoing receipts.  ARCHIVE-ID is preserved for bodyless archive
stanzas whose downstream handlers need the MAM result id.  Mutates
OUTER in place."
  (setcar (cdr outer) (append (jabber-xml-node-attributes inner)
                              `((jabber-mam--origin . "t")
                                ,@(and archive-id
                                       `((jabber-mam--archive-id
                                          . ,archive-id))))))
  (setcdr (cdr outer) (cddr inner)))

(defun jabber-mam--session-predicate (jc)
  "Return a predicate for JC's captured transport, stream and account.
Ordinary FSM state copies do not retire this session."
  (let* ((state (fsm-get-state-data jc))
         (connection (plist-get state :connection))
         (stream (plist-get state :session-id))
         (username (plist-get state :username))
         (server (plist-get state :server)))
    (lambda ()
      (let ((current (fsm-get-state-data jc)))
        (and (eq connection (plist-get current :connection))
             (equal stream (plist-get current :session-id))
             (equal username (plist-get current :username))
             (equal server (plist-get current :server)))))))

(defun jabber-mam--current-query-p (jc query)
  "Return non-nil when JC still owns the registered QUERY."
  (and (eq jc (plist-get query :jc))
       (memq query jabber-mam--syncing)
       (funcall (plist-get query :current-p))))

(defun jabber-mam--valid-sender-p (jc from query)
  "Return non-nil when FROM is QUERY's archive on its owning JC.
Require the exact archive JID, never a room occupant or account resource.
For compatibility, absent FROM is accepted for an owned personal archive."
  (and (jabber-mam--current-query-p jc query)
       (let ((target (plist-get query :to)))
         (if from
             (equal from (or target (jabber-connection-bare-jid jc)))
           (null target)))))

(defun jabber-mam--classify-direction (jc from to type)
  "Classify message direction and peer from MAM result fields.
JC is the connection, FROM/TO are stanza JIDs, TYPE is message type.
Return (DIRECTION . PEER) where DIRECTION is \"in\" or \"out\"."
  (let* ((our-jid (jabber-connection-bare-jid jc))
         (groupchat-p (string= type "groupchat"))
         (direction (if groupchat-p
			(let ((nick (jabber-jid-resource from))
                              (room (jabber-jid-user from)))
                          (if (and nick
                                   (jabber-mam--our-muc-nick-p
                                    room nick jc))
                              "out" "in"))
                      (if (string= (jabber-jid-user from) our-jid)
                          "out" "in")))
         (peer (if groupchat-p
                   (jabber-jid-user from)
                 (jabber-jid-user
                  (if (string= direction "out") to from)))))
    (cons direction peer)))

(defun jabber-mam--extract-fields (jc inner-msg stamp)
  "Extract message fields from INNER-MSG for storage.
JC is the connection.  STAMP is the MAM delay timestamp string.
Returns a plist with :from :to :type :body :stanza-id :our-jid
:direction :peer :timestamp :oob-entries, or nil if direction
cannot be determined."
  (let* ((from (jabber-xml-get-attribute inner-msg 'from))
         (to (jabber-xml-get-attribute inner-msg 'to))
         (type (or (jabber-xml-get-attribute inner-msg 'type) "chat"))
         (body-el (car (jabber-xml-get-children inner-msg 'body)))
         (body (and body-el (car (jabber-xml-node-children body-el))))
         (stanza-id (jabber-xml-get-attribute inner-msg 'id))
         (our-jid (jabber-connection-bare-jid jc))
         (dir-peer (jabber-mam--classify-direction jc from to type))
         (direction (car dir-peer))
         (peer (cdr dir-peer))
         (timestamp (and stamp (jabber-parse-time stamp)))
         (oob-entries (jabber-db--extract-oob-entries inner-msg)))
    (list :from from :to to :type type :body body
          :stanza-id stanza-id :our-jid our-jid
          :direction direction :peer peer
          :timestamp timestamp :oob-entries oob-entries)))

(defun jabber-mam--store-new-message-p (jc inner-msg)
  "Return non-nil when INNER-MSG should be stored as a new message for JC."
  (not (run-hook-with-args-until-success
        'jabber-history-inhibit-received-message-functions
        jc inner-msg)))

(defun jabber-mam--message-action (jc inner-msg fields)
  "Return how MAM should handle INNER-MSG with FIELDS for JC."
  (cond
   ((or (not (plist-get fields :peer))
        (not (plist-get fields :body)))
    :unwrap)
   ((jabber-message-correct--replace-id inner-msg) :correct)
   ((jabber-mam--store-new-message-p jc inner-msg) :store)
   (t :unwrap)))

(defun jabber-mam--page-failed (query)
  "Prevent QUERY's current page from publishing a durable cursor."
  (plist-put query :failed t))

(defun jabber-mam--process-message (jc xml-data)
  "Process JC's owned MAM result XML-DATA in its own short transaction.
Consume the owned envelope so live handlers never see archived content.
Malformed or failing results fail the page instead of being skipped."
  (let* ((result (jabber-xml-child-with-xmlns xml-data jabber-mam-xmlns))
         (qid (jabber-xml-get-attribute result 'queryid))
         (query (cl-find qid jabber-mam--syncing
                         :key (lambda (entry) (plist-get entry :id))
                         :test #'equal))
         (retired (and (not query) qid
                       (cl-find-if (lambda (entry) (member qid (plist-get entry :retired-ids)))
                                   jabber-mam--syncing))))
    ;; A late result of an attempt the watchdog replaced: consumed, unused.
    (when (and retired
               (jabber-mam--valid-sender-p
                jc (jabber-xml-get-attribute xml-data 'from) retired))
      (setcdr (cdr xml-data) nil))
    (when (and query (plist-get query :page)
               (jabber-mam--valid-sender-p
                jc (jabber-xml-get-attribute xml-data 'from) query))
      (unwind-protect
          (condition-case err
              (if (or (not (eq (jabber-xml-node-name result) 'result))
                      (not (jabber-mam--parse-result xml-data))
                      (not (stringp (jabber-xml-get-attribute result 'id)))
                      (string-empty-p (jabber-xml-get-attribute result 'id)))
                  (jabber-mam--page-failed query)
                (plist-put query :received (1+ (or (plist-get query :received) 0)))
                (plist-put query :total (1+ (or (plist-get query :total) 0)))
                (plist-put query :heard-at (float-time))
                (jabber-mam--commit-result jc query (copy-tree xml-data)))
            ((error quit)
             (jabber-mam--page-failed query)
             (signal (car err) (cdr err))))
        (setcdr (cdr xml-data) nil)))))

(defun jabber-mam--commit-result (jc query stanza)
  "Process QUERY's archived STANZA for JC in one short transaction.
Decrypt and store it, run an unwrapped bodyless action through the
message chain, then COMMIT.  Any error, including one a chain handler
raises, aborts the transaction.  So does retirement of QUERY meanwhile.
Otherwise roll back and restore in-memory state the transaction
consumed or changed, so a retry starts again from the restored durable
state.  Effects deferred with `jabber-chat--on-decrypt-commit' run only
after COMMIT.  In a batch (`jabber-mam--batch') the result is a
savepoint of the batch's transaction instead, and its effects wait for
the batch's COMMIT."
  (let* ((db (jabber-db-ensure-open))
         (batch (and db jabber-mam--batch (jabber-mam--batch-open db)))
         (jabber-chat--decrypt-undo (list #'ignore))
         (jabber-chat--decrypt-commit nil)
         (jabber-mam--effects (and jabber-mam-committed-functions (list nil)))
         (committed nil))
    (when db
      ;; Immediate: see `jabber-db--with-savepoint'.
      (sqlite-execute db (if batch "SAVEPOINT jabber_mam_result" "BEGIN IMMEDIATE")))
    (unwind-protect
        (let ((jabber-db--in-transaction db))
          (jabber-mam--process-owned-message jc stanza)
          (when (jabber-xml-get-attribute stanza 'jabber-mam--origin)
            (jabber-process-input
             jc stanza (lambda (err) (signal (car err) (cdr err)))))
          (when (jabber-mam--current-query-p jc query)
            (when db
              (sqlite-execute db (if batch "RELEASE jabber_mam_result" "COMMIT")))
            (setq committed t)
            (let ((effects (jabber-mam--result-effects
                            jabber-mam--effects jabber-chat--decrypt-commit)))
              (if batch
                  (jabber-mam--batch-add query effects jabber-chat--decrypt-undo)
                (let ((jabber-db--in-transaction nil))
                  (funcall effects))))))
      (unless committed
        (when db
          (ignore-error sqlite-error
            (if (not batch)
                (sqlite-execute db "ROLLBACK")
              (sqlite-execute db "ROLLBACK TO jabber_mam_result")
              (sqlite-execute db "RELEASE jabber_mam_result"))))
        (mapc #'jabber-lifecycle--call-contained jabber-chat--decrypt-undo)))
    (when (and batch (jabber-mam--batch-due-p))
      (jabber-mam-batch-commit))))

(defun jabber-mam--result-effects (effects commits)
  "A function running a committed result's EFFECTS, then its decrypt COMMITS.
EFFECTS is `jabber-mam--effects' as the result left it."
  (lambda ()
    (when effects
      (jabber-lifecycle--dispatch-contained
       'jabber-mam-committed-functions (cdr effects)))
    (dolist (effect (reverse commits))
      (jabber-lifecycle--call-contained effect))))

;;; Batched results: one transaction for many

(defvar jabber-mam-batch-size 100
  "Most results a batch holds before it commits.")

(defvar jabber-mam-batch-age 0.05
  "Seconds a batch stays open at most before its next result commits it.")

(defun jabber-mam-make-batch ()
  "A closed batch for `jabber-mam--batch'."
  (list :open nil :count 0 :effects nil :undos nil :queries nil :opened nil :gc nil))

(defun jabber-mam--batch-open (db)
  "Open the batch's transaction on DB unless it is open; return non-nil."
  (unless (plist-get jabber-mam--batch :open)
    (sqlite-execute db "BEGIN IMMEDIATE")
    (plist-put jabber-mam--batch :open db)
    ;; Statements now hold the lock: none waits for it (`jabber-db--begin',
    ;; `jabber-db--acquires-lock-p').
    (setq jabber-db--in-transaction db)
    (plist-put jabber-mam--batch :gc gc-cons-threshold)
    (setq gc-cons-threshold most-positive-fixnum)
    (plist-put jabber-mam--batch :opened (float-time)))
  t)

(defun jabber-mam--batch-note (query)
  "Count QUERY among the open batch's queries, if a batch is open."
  (when (and jabber-mam--batch (plist-get jabber-mam--batch :open)
             (not (memq query (plist-get jabber-mam--batch :queries))))
    (plist-put jabber-mam--batch :queries
               (cons query (plist-get jabber-mam--batch :queries)))))

(defun jabber-mam--batch-add (query effects undos)
  "Add QUERY's committed result to the open batch: its EFFECTS and UNDOS."
  (jabber-mam--batch-note query)
  (plist-put jabber-mam--batch :effects (cons effects (plist-get jabber-mam--batch :effects)))
  (plist-put jabber-mam--batch :undos (cons undos (plist-get jabber-mam--batch :undos)))
  (plist-put jabber-mam--batch :count (1+ (plist-get jabber-mam--batch :count))))

(defun jabber-mam--batch-due-p ()
  "Non-nil when the open batch should commit now: full, old, or the session waits."
  (or (>= (plist-get jabber-mam--batch :count) jabber-mam-batch-size)
      (> (- (float-time) (plist-get jabber-mam--batch :opened)) jabber-mam-batch-age)
      (when-let* ((file (jabber-db--intent-file)))
        (jabber-db--intent-fresh-p file))))

(defun jabber-mam-batch-commit ()
  "Commit the open batch, then run its results' effects in order.
Should the COMMIT fail, the batch rolls back, its results' in-memory
changes are undone, and its queries fail: their cursors moved inside it."
  (when-let* ((batch jabber-mam--batch)
              (db (plist-get batch :open)))
    (let ((effects (reverse (plist-get batch :effects)))
          (undos (plist-get batch :undos))
          (queries (plist-get batch :queries)))
      (setq gc-cons-threshold (or (plist-get batch :gc) gc-cons-threshold))
      (dolist (key '(:open :effects :undos :queries :opened :gc))
        (plist-put batch key nil))
      (plist-put batch :count 0)
      (setq jabber-db--in-transaction nil)
      (if (condition-case err
              (progn (sqlite-execute db "COMMIT") t)
            (error
             (ignore-error sqlite-error (sqlite-execute db "ROLLBACK"))
             (dolist (undo undos)
               (mapc #'jabber-lifecycle--call-contained undo))
             (dolist (query queries)
               (when (memq query jabber-mam--syncing)
                 (jabber-mam--fail-query
                  query (format "its results could not be stored: %s"
                                (error-message-string err)))))
             nil))
          (mapc #'funcall effects)))))

(defun jabber-mam--message-identity (jc query inner-msg archive-id)
  "Return accepted QUERY's identity for INNER-MSG and ARCHIVE-ID on JC.
A room archive UID supplies missing room identity only for an owned query
whose exact target is the forwarded message's room.  The caller must have
validated the outer sender.  Explicit room stanza IDs take precedence;
never promote a personal or foreign archive UID to room identity."
  (let* ((archive (or (plist-get query :to)
                      (jabber-connection-bare-jid jc)))
         (identity (jabber-db--message-identity inner-msg archive archive-id)))
    (when (and (not (plist-get identity :room-id))
               (jabber-mam--current-query-p jc query)
               (equal (jabber-xml-get-attribute inner-msg 'type) "groupchat")
               (stringp archive-id) (not (string-empty-p archive-id))
               (plist-get query :to)
               (equal (plist-get query :to)
                      (jabber-jid-user
                       (jabber-xml-get-attribute inner-msg 'from))))
      (setq identity (plist-put identity :room-id archive-id)))
    identity))

(defun jabber-mam--process-owned-message (jc xml-data)
  "Handle a MAM result <message> from the message chain.
JC is the Jabber connection.  XML-DATA is the stanza."
  (when-let* ((result-el (jabber-xml-child-with-xmlns
                          xml-data jabber-mam-xmlns))
              (qid (jabber-xml-get-attribute result-el 'queryid))
              (query (cl-find qid jabber-mam--syncing
                              :key (lambda (entry) (plist-get entry :id))
                              :test #'equal))
              (page (plist-get query :page))
              ((jabber-mam--valid-sender-p
                jc (jabber-xml-get-attribute xml-data 'from) query))
              (parsed (jabber-mam--parse-result xml-data)))
    (let* ((archive-id (nth 0 parsed))
           (stamp (nth 1 parsed))
           (inner-msg (nth 2 parsed))
           (encrypted (jabber-xml-encrypted-p inner-msg))
           (inner-msg
            (let ((inhibit-message t)
                  (message-log-max nil))
              (jabber-chat--decrypt-if-needed jc inner-msg)))
           (fields (jabber-mam--extract-fields jc inner-msg stamp))
           (peer (plist-get fields :peer))
           (body (plist-get fields :body))
           (timestamp (and-let* ((time (plist-get fields :timestamp)))
                        (floor (float-time time))))
           (action (jabber-mam--message-action jc inner-msg fields)))
      (when (jabber--decrypt-failure-body-p body)
        (jabber-mam--page-failed query))
      (when (and timestamp (or (null (plist-get query :oldest))
                               (< timestamp (plist-get query :oldest))))
        (plist-put query :oldest timestamp))
      (when (and timestamp (or (null (plist-get query :latest))
                               (> timestamp (plist-get query :latest))))
        (plist-put query :latest timestamp))
      (when (and (eq page (plist-get query :page))
                 (jabber-mam--current-query-p jc query))
        (pcase action
          ((or :correct :store)
           (pcase action
             (:correct
              (unless (jabber--decrypt-failure-body-p body)
                (jabber-message-correct--apply
                 (jabber-message-correct--replace-id inner-msg)
                 body (plist-get fields :from)
                 (string= (plist-get fields :type) "groupchat") nil
                 (jabber-db--extract-occupant-id inner-msg)
                 (plist-get fields :our-jid) peer nil)))
             (:store
              (let ((jabber-db-message-thread-stored-functions nil)
                    (identity (jabber-mam--message-identity
                               jc query inner-msg archive-id)))
                (jabber-db-store-message
                 (plist-get fields :our-jid) peer
                 (plist-get fields :direction) (plist-get fields :type)
                 body timestamp (jabber-jid-resource (plist-get fields :from))
                 (plist-get fields :stanza-id)
                 (if (equal (plist-get fields :type) "groupchat")
                     (plist-get identity :room-id)
                   archive-id)
                 (jabber-db--extract-occupant-id inner-msg)
                 (plist-get fields :oob-entries) encrypted
                 (jabber-db--extract-reply-fields inner-msg)
                 (jabber-db--extract-thread-fields inner-msg)
                 identity))))
           (jabber-mam--mark-dirty jc peer (plist-get fields :type))
           (setcdr (cdr xml-data) nil))
          (:unwrap
           ;; A bodyless action (reaction, retraction, marker) may change
           ;; PEER's displayed history without adding a row.
           (when peer
             (jabber-mam--note-effect (jabber-connection-bare-jid jc) peer
                                      (plist-get fields :type)))
           (jabber-mam--unwrap-into xml-data inner-msg archive-id)))))))

(defun jabber-mam--our-muc-nick-p (room nick jc)
  "Return non-nil if NICK in ROOM is us on connection JC.
Checks the current room nickname first, then falls back to
comparing with the account username to handle nick changes."
  (require 'jabber-muc)
  (or (and-let* ((my-nick (or (and jabber-mam--room-nicks
                                  (gethash room jabber-mam--room-nicks))
                              (jabber-muc-nickname room jc))))
        (string= nick my-nick))
      (string= nick (plist-get (fsm-get-state-data jc) :username))))


(defvar jabber-mam--dirty-index nil
  "(LIST . HASH): the members of `jabber-mam--dirty-peers' while it is LIST.
A list set from elsewhere (drained, or bound by a caller) is indexed
again at the next mark, so marking a stored result costs no scan.")

(defun jabber-mam--mark-dirty (jc peer type)
  "Record that PEER's buffer needs redisplay after sync.
JC identifies the local account.  TYPE is the message type."
  (let ((entry (list (jabber-connection-bare-jid jc) peer type)))
    (jabber-mam--note-effect (car entry) peer type)
    (unless (and jabber-mam--dirty-index
                 (eq (car jabber-mam--dirty-index) jabber-mam--dirty-peers))
      (let ((index (make-hash-table :test #'equal)))
        (dolist (member jabber-mam--dirty-peers) (puthash member t index))
        (setq jabber-mam--dirty-index (cons jabber-mam--dirty-peers index))))
    (unless (gethash entry (cdr jabber-mam--dirty-index))
      (puthash entry t (cdr jabber-mam--dirty-index))
      (push entry jabber-mam--dirty-peers)
      (setcar jabber-mam--dirty-index jabber-mam--dirty-peers))))


(defun jabber-mam--redraw-dirty ()
  "Signal that accumulated dirty peers need display refresh.
Drains `jabber-mam--dirty-peers' and runs
`jabber-mam-sync-complete-functions'."
  (let ((peers (prog1 jabber-mam--dirty-peers
                 (setq jabber-mam--dirty-peers nil))))
    (when peers
      (jabber-lifecycle--dispatch-contained
       'jabber-mam-sync-complete-functions peers))))

(defun jabber-mam--redraw-committed ()
  "Refresh open buffers of peers whose pages have committed.
Unlike `jabber-mam--redraw-dirty', this does not signal completion, and
it skips buffers whose reader is scrolled into history; the completion
refresh covers those."
  (setq jabber-mam--redraw-timer nil)
  (when-let* ((peers (prog1 jabber-mam--redraw-peers
                       (setq jabber-mam--redraw-peers nil))))
    (jabber-lifecycle--call-contained
     #'jabber-chat--handle-mam-sync-complete peers t)))

(defvar jabber-mam--redraw-index nil
  "(LIST . HASH): the members of `jabber-mam--redraw-peers' while it is LIST.")

(defun jabber-mam--schedule-redraw (peers)
  "Refresh committed PEERS' buffers after `jabber-mam--redraw-delay'.
Pages committed while the timer is pending share its redraw, so a long
catch-up shows its progress without rebuilding buffers for every page."
  (unless (and jabber-mam--redraw-index
               (eq (car jabber-mam--redraw-index) jabber-mam--redraw-peers))
    (let ((index (make-hash-table :test #'equal)))
      (dolist (member jabber-mam--redraw-peers) (puthash member t index))
      (setq jabber-mam--redraw-index (cons jabber-mam--redraw-peers index))))
  (dolist (entry peers)
    (unless (gethash entry (cdr jabber-mam--redraw-index))
      (puthash entry t (cdr jabber-mam--redraw-index))
      (push entry jabber-mam--redraw-peers)))
  (setcar jabber-mam--redraw-index jabber-mam--redraw-peers)
  (unless jabber-mam--redraw-timer
    (setq jabber-mam--redraw-timer
          (run-with-timer jabber-mam--redraw-delay nil
                          #'jabber-mam--redraw-committed))))

;;; Page settlement

(defun jabber-mam--accept-page (query last-id)
  "Advance QUERY's forward cursor to LAST-ID after its page's results.
Each result has already committed in its own transaction, so this is
one autocommitted write (inside an open batch, part of it); then
schedule a redraw of committed peers."
  (when last-id
    (plist-put query :last-uid last-id))
  (jabber-mam--batch-note query)
  (when (plist-get query :forward)
    (let ((expected (plist-get query :cursor-expected))
          (uid (or last-id (plist-get query :cursor-expected))))
      (when-let* ((db (jabber-db-ensure-open)))
        (jabber-db--advance-mam-progress
         db (plist-get query :account) (plist-get query :archive)
         (plist-get query :with) expected uid
         (plist-get query :coverage-start)))
      (plist-put query :cursor-expected uid)))
  (when jabber-mam--dirty-peers
    (jabber-mam--schedule-redraw jabber-mam--dirty-peers)))

;;; Query and pagination

(defun jabber-mam--catch-up-page-size (&optional limit)
  "Results a catch-up page asks for, at most LIMIT when it is non-nil."
  (let ((size (max 1 jabber-mam-page-size jabber-mam-catch-up-min-page-size)))
    (if limit (max 1 (min size limit)) size)))

(defun jabber-mam--query (jc &optional after-id queryid with start to
                             before-id max callback result-callback limit
                             extra)
  "Start a MAM query on JC, paginating from AFTER-ID.
QUERYID correlates results; generate it if nil.  WITH and START are filters.
TO is nil for the personal archive, or a room bare JID.
BEFORE-ID requests one backward page of MAX messages.
Call CALLBACK without arguments once on settlement, including failure.
Then call RESULT-CALLBACK once with `success', `failed' or `cancelled'.
With LIMIT and BEFORE-ID t, page back from the archive's newest message
until LIMIT results arrived or the archive's start; the query records
the newest UID as :newest and the oldest message time as :oldest.
EXTRA is a plist of query properties set over the computed ones:
`:end', an XEP-0082 time the results stop at (it may be moved between
pages); `:forward', nil to keep the query from advancing the stored
cursor; `:on-page', a function called with the query after each page
that leaves it incomplete, before the next is requested.  Every query
records its last page's last UID as :last-uid and its newest message
time as :latest.
Success means accepted query completion, including a bounded backward page,
not complete archive coverage.  Each result commits on arrival in its own
transaction; a page's fin then advances the forward cursor.
Failure means a permanent IQ error, local send error, a result that failed
or could not commit, or incomplete forward pagination without cursor
progress.  Cancellation means retirement
of the room, connection or session, or caller quit (which is re-signaled).
Intermediate pages and the first stale-cursor retry do not settle the query.
Retire ownership before attempting each callback independently; contain their
errors and quits without changing the outcome.  Callbacks can run before this
function returns, including on synchronous send failure.
Return the owned query record."
  (dolist (filter (list with to))
    (unless (or (null filter) (and (stringp filter) (not (string-empty-p filter))))
      (error "Invalid MAM archive or coverage filter")))
  (let* ((account (jabber-connection-bare-jid jc))
         (archive (or to account))
         (progress (jabber-db-mam-progress account archive with))
         (query (list :id (or queryid (jabber-mam--make-queryid)) :jc jc
                     :current-p (jabber-mam--session-predicate jc)
                     :with with :start start :to to :before before-id
                     :max (or max (jabber-mam--catch-up-page-size limit))
                     :after after-id :limit limit :total 0 :newest nil :oldest nil
                     :callback callback :result-callback result-callback
                     :page nil :timer nil :iq-id nil
                     :retried nil :failed nil :received 0
                     :due-at nil :heard-at nil :silent nil :error nil :retired-ids nil
                     :end nil :on-page nil :last-uid nil :latest nil
                     :account account :archive archive
                     :cursor-expected (car progress)
                     :coverage-start (if progress (cadr progress) start)
                     :forward (and (not before-id)
                                   (equal after-id (car progress))
                                   (or (null progress)
                                       (equal start (cadr progress)))))))
    (cl-loop for (key value) on extra by #'cddr
             do (plist-put query key value))
    (push query jabber-mam--syncing)
    (jabber-mam--send-page query)
    query))

(defun jabber-mam--schedule-page (query)
  "Request QUERY's next page after `jabber-mam-page-delay' seconds.
At 0, request it now.  It is due then; should the request never be
made, the watchdog makes it (`jabber-mam-check-stalls')."
  (plist-put query :due-at (+ (float-time) jabber-mam-page-delay))
  (jabber-mam--watch)
  (if (<= jabber-mam-page-delay 0)
      (jabber-mam--send-page query)
    (plist-put query :timer
               (run-with-timer jabber-mam-page-delay nil #'jabber-mam--send-page query))))

(defun jabber-mam--send-page (query)
  "Send the next page of QUERY only while its owner remains current."
  (when (and (memq query jabber-mam--syncing)
             (not (plist-get query :page)))
    (plist-put query :timer nil)
    (if (not (jabber-mam--current-query-p (plist-get query :jc) query))
        (jabber-mam--complete-query query 'cancelled)
      (let ((page (list nil)))
        (plist-put query :page page)
        (plist-put query :received 0)
        (plist-put query :due-at nil)
        (plist-put query :heard-at (float-time))
        (plist-put query :iq-id (jabber-mam--make-queryid))
        (jabber-mam--watch)
        (condition-case err
            (progn
              (jabber-send-iq
               (plist-get query :jc) (plist-get query :to) "set"
               (jabber-mam--build-query
                (plist-get query :id) (plist-get query :with)
                (plist-get query :start) (plist-get query :after)
                (plist-get query :max) (plist-get query :before)
                (plist-get query :end))
               #'jabber-mam--handle-fin (cons query page)
               #'jabber-mam--handle-error (cons query page)
               (plist-get query :iq-id)
               (lambda (jc xml)
                 (and (eq page (plist-get query :page))
                      (jabber-mam--valid-sender-p
                       jc (jabber-xml-get-attribute xml 'from) query)))))
          (quit
           (jabber-mam--complete-query query 'cancelled)
           (signal (car err) (cdr err)))
          (error
           (jabber-mam--fail-query
            query (format "the request could not be sent: %s"
                          (error-message-string err)))))))))

(defun jabber-mam--reply-query (jc xml-data closure)
  "Return the active query owned by JC for XML-DATA and CLOSURE."
  (let ((query (car closure)))
    (when (and (eq (cdr closure) (plist-get query :page))
               (jabber-mam--valid-sender-p
                jc (jabber-xml-get-attribute xml-data 'from) query))
      query)))

(defun jabber-mam--handle-fin (jc xml-data closure)
  "Settle or paginate JC's owned page from XML-DATA and CLOSURE."
  (when-let* ((query (jabber-mam--reply-query jc xml-data closure)))
    (condition-case err
        (let* ((fin (jabber-mam--parse-fin xml-data))
               (last-id (plist-get fin :last)))
          (cond
           ((jabber-mam--fin-problem query fin)
            (jabber-mam--fail-query query (jabber-mam--fin-problem query fin)))
           (t
            (plist-put query :silent nil)
            (jabber-mam--accept-page query last-id)
            (let ((before (plist-get query :before))
                  (limit (plist-get query :limit))
                  (total (or (plist-get query :total) 0)))
              (when (and before last-id (not (plist-get query :newest)))
                (plist-put query :newest last-id))
              (cond
               ((or (plist-get fin :complete)
                    (and before (or (not limit) (>= total limit)
                                    (null (plist-get fin :first)))))
                ;; Stored before it is said to have succeeded: should its
                ;; batch fail to commit, the query fails instead.
                (jabber-mam-batch-commit)
                (when (memq query jabber-mam--syncing)
                  (jabber-mam--complete-query query 'success)))
               (t
                (jabber-mam--release-page query)
                (if before
                    (progn
                      (plist-put query :before (plist-get fin :first))
                      (plist-put query :max (jabber-mam--catch-up-page-size (- limit total))))
                  (plist-put query :after last-id))
                (when-let* ((on-page (plist-get query :on-page)))
                  (jabber-lifecycle--call-contained on-page query))
                (jabber-mam--schedule-page query)))))))
      ((error quit)
       (jabber-mam--page-failed query)
       (jabber-mam--fail-query query (format "its answer could not be read: %s"
                                             (error-message-string err)))
       (signal (car err) (cdr err))))))

(defun jabber-mam--fin-problem (query fin)
  "Why QUERY cannot go on from its page's FIN, or nil when it can.
FIN is `jabber-mam--parse-fin''s, nil for an answer without a valid one."
  (let ((last-id (plist-get fin :last)))
    (cond ((not fin) "the archive answered without a valid <fin>")
          ((plist-get query :failed) "a result could not be stored")
          ((and (> (or (plist-get query :received) 0) 0) (null last-id))
           "a page of results came without an RSM <last>")
          ((and (not (plist-get fin :complete)) (not (plist-get query :before))
                (null last-id))
           "an incomplete page came without an RSM <last>")
          ((and (not (plist-get fin :complete)) (not (plist-get query :before))
                (equal last-id (plist-get query :after)))
           "the archive's cursor did not move"))))

(defun jabber-mam--handle-error (jc xml-data closure)
  "Settle JC's failed page from XML-DATA and CLOSURE.
Retry a stale forward cursor once without that cursor, preserving filters."
  (when-let* ((query (jabber-mam--reply-query jc xml-data closure)))
    (let ((error-el (car (jabber-xml-get-children xml-data 'error))))
      (if (and (plist-get query :after)
               (not (plist-get query :before))
               (not (plist-get query :retried))
               (car (jabber-xml-get-children error-el 'item-not-found)))
          (progn
            (jabber-mam--release-page query)
            (plist-put query :after nil)
            (plist-put query :retried t)
            (jabber-mam--send-page query))
        (jabber-mam--fail-query
         query (format "the archive answered %s"
                       (or (ignore-errors
                             (symbol-name (jabber-xml-node-name
                                           (car (jabber-xml-node-children error-el)))))
                           "an error")))))))

;;; Post-connect catch-up

(defun jabber-mam--initial-start ()
  "Return the configured initial catch-up timestamp, or nil."
  (when jabber-mam-catch-up-days
    (format-time-string
     "%Y-%m-%dT%H:%M:%SZ"
     (time-subtract (current-time) (* jabber-mam-catch-up-days 86400)) t)))

(defun jabber-mam--catch-up (jc &optional done)
  "Sync missed messages for JC via MAM; call DONE with the outcome."
  (let* ((account (jabber-connection-bare-jid jc))
         (progress (jabber-db-mam-progress account account nil)))
    (jabber-mam--query jc (car progress) nil nil (cadr progress)
                       nil nil nil nil done)))

(defun jabber-mam--background-p ()
  "Non-nil when catch-ups run in a worker VM (jabber-mam-sync.el)."
  (and (fboundp 'jabber-mam-sync-available-p) (jabber-mam-sync-available-p)))

(declare-function jabber-mam-sync-request "jabber-mam-sync" (jc &optional room with))

(defun jabber-mam-maybe-catchup (jc)
  "Post-connect hook on JC: sync messages via MAM if enabled.
Added to `jabber-post-connect-hooks'."
  (cond
   ((not jabber-mam-enable))
   ((jabber-mam--background-p) (jabber-mam-sync-request jc))
   (t
    (jabber-disco-get-info
     jc (jabber-connection-bare-jid jc) nil
     (lambda (jc _closure-data result)
       (when (and (listp result)
                  (not (eq (car result) 'error))
                  (member jabber-mam-xmlns (cadr result)))
         (jabber-mam--catch-up jc)))
     nil))))

;;; 1:1 chat MAM catch-up

(defun jabber-mam--chat-catch-up (jc peer token)
  "Sync PEER's archive on JC for automatic catch-up TOKEN."
  (let* ((account (jabber-connection-bare-jid jc))
         (progress (jabber-db-mam-progress account account peer))
         (last-id (car progress))
         (start (cadr progress)))
    (jabber-mam--query
     jc last-id nil peer start nil nil nil
     (lambda () (jabber-mam--finish-peer-sync jc peer token)))))

(defun jabber-mam--handle-chat-disco (jc closure-data result)
  "Start a peer catch-up after disco completes.
JC is the connection.  CLOSURE-DATA is (PEER TOKEN).  RESULT is
the disco response."
  (pcase-let ((`(,peer ,token) closure-data))
    (when (jabber-mam--peer-sync-active-p jc peer token)
      (if (and (listp result)
               (not (eq (car result) 'error))
               (member jabber-mam-xmlns (cadr result)))
          (condition-case err
              (jabber-mam--chat-catch-up jc peer token)
            (error
             (jabber-mam--finish-peer-sync jc peer token)
             (message "MAM: peer catch-up failed to start: %s"
                      (error-message-string err))))
        (jabber-mam--finish-peer-sync jc peer token)))))

(defun jabber-mam-chat-opened (jc peer)
  "Trigger 1:1 MAM catch-up when opening a chat with PEER.
JC is the Jabber connection.  Called from `jabber-chat-with'.
Sets the syncing indicator immediately; clears it when the catch-up
query completes (or when disco reveals MAM is not supported)."
  (if (and jabber-mam-enable (jabber-mam--background-p))
      (jabber-mam-sync-request jc nil peer)
    (jabber-mam--chat-opened-in-session jc peer)))

(defun jabber-mam--chat-opened-in-session (jc peer)
  "Catch up JC's history with PEER in the session, with an indicator."
  (when-let* ((token (and jabber-mam-enable
                          (jabber-mam--begin-peer-sync jc peer))))
    (jabber-mam--signal-peer-syncing jc peer "chat" t)
    (condition-case err
        (jabber-disco-get-info
         jc (jabber-connection-bare-jid jc) nil
         #'jabber-mam--handle-chat-disco (list peer token))
      (error
       (jabber-mam--finish-peer-sync jc peer token)
       (signal (car err) (cdr err))))))

;;; MUC MAM catch-up

(defun jabber-mam--room-history (jc group done)
  "Fetch GROUP's last `jabber-mam-room-history' messages on JC.
Then start GROUP's cursor at the newest of them and its coverage at the
oldest, unless a cursor appeared meanwhile; call DONE with the outcome.
An interrupted fetch records no cursor: the next catch-up fetches the
same newest messages again, which the archive identities deduplicate."
  (let (query)
    (setq query
          (jabber-mam--query
           jc nil nil nil nil group t nil nil
           (lambda (outcome)
             (when (and (eq outcome 'success) query (plist-get query :newest))
               (jabber-db-seed-mam-progress
                (jabber-connection-bare-jid jc) group nil
                (plist-get query :newest)
                (and (plist-get query :oldest)
                     (format-time-string "%Y-%m-%dT%H:%M:%SZ"
                                         (plist-get query :oldest) t))))
             (funcall done outcome))
           (max 1 jabber-mam-room-history)))))

(defun jabber-mam--muc-catch-up (jc group &optional done)
  "Sync GROUP's archive on JC, then clear its syncing indicator.
A room with no cursor gets only its last `jabber-mam-room-history'
messages and a cursor there; one with a cursor catches up forward from
it, completely.  Call DONE with the outcome."
  (let* ((account (jabber-connection-bare-jid jc))
         (progress (jabber-db-mam-progress account group nil))
         (finish (lambda (outcome)
                   (jabber-mam--signal-peer-syncing jc group "groupchat" nil)
                   (when done (funcall done outcome)))))
    (if (null (car progress))
        (jabber-mam--room-history jc group finish)
      (jabber-mam--query
       jc (car progress) nil nil (cadr progress) group nil nil nil finish))))

(defun jabber-mam-muc-joined (jc group)
  "Trigger MUC MAM catch-up after joining GROUP.
JC is the Jabber connection.  Called from MUC self-presence handler.
Sets the syncing indicator immediately; clears it when the catch-up
query completes (or when disco reveals MAM is not supported)."
  (cond
   ((not jabber-mam-enable))
   ((jabber-mam--background-p) (jabber-mam-sync-request jc group))
   (t
    (jabber-mam--signal-peer-syncing jc group "groupchat" t)
    (jabber-disco-get-info
     jc group nil
     (lambda (jc closure-data result)
       (let ((group (car closure-data)))
         (if (and (listp result)
                  (not (eq (car result) 'error))
                  (member jabber-mam-xmlns (cadr result)))
             (jabber-mam--muc-catch-up jc group)
           (jabber-mam--signal-peer-syncing jc group "groupchat" nil))))
     (list group)))))

(defun jabber-mam-sync-buffer ()
  "Sync recent messages from this buffer's server archive.
Fetch at most `jabber-chat-buffer-msg-count' messages.  Store new messages
and update existing messages through deduplication, including successful
re-decryption of placeholders.  Never infer deletion from archive absence:
a bounded page, even a successful one, is not evidence of remote deletion.
Refresh the buffer after settlement."
  (interactive)
  (unless (memq jabber-buffer-connection jabber-connections)
    (user-error "Not connected"))
  (let* ((jc jabber-buffer-connection)
         (count (jabber-chat-buffer-msg-count))
         (group (bound-and-true-p jabber-group))
         (peer (or group
                   (jabber-jid-user (bound-and-true-p jabber-chatting-with))))
         (type (if group "groupchat" "chat")))
    (jabber-mam--signal-peer-syncing jc peer type t)
    (jabber-mam--mark-dirty jc peer type)
    (jabber-mam--query
     jc nil nil (unless group peer) nil group t count
     (lambda ()
       (jabber-mam--signal-peer-syncing jc peer type nil)))))

;;; Disconnect cleanup

(defun jabber-mam--cleanup-connection (jc)
  "Retire JC's queries and automatic catch-ups on disconnect."
  (dolist (query (copy-sequence jabber-mam--syncing))
    (when (eq jc (plist-get query :jc))
      (jabber-mam--complete-query query 'cancelled)))
  (dolist (entry (copy-sequence jabber-mam--peer-syncing))
    (when (eq (caar entry) jc)
      (jabber-mam--finish-peer-sync jc (cadar entry) (cdr entry)))))

(defun jabber-mam--cleanup-all ()
  "Retire all MAM queries and automatic catch-ups on disconnect."
  (dolist (query (copy-sequence jabber-mam--syncing))
    (jabber-mam--complete-query query 'cancelled))
  (dolist (entry (copy-sequence jabber-mam--peer-syncing))
    (jabber-mam--finish-peer-sync (caar entry) (cadar entry) (cdr entry))))

;;; MUC query cancellation

(defun jabber-mam--cancel-muc-query (room &optional jc)
  "Retire MAM queries and pending pagination for ROOM on JC.
When JC is nil, intentionally retire this room's queries on all accounts."
  (dolist (query (copy-sequence jabber-mam--syncing))
    (when (and (equal room (plist-get query :to))
               (or (null jc) (eq jc (plist-get query :jc))))
      (jabber-mam--complete-query query 'cancelled))))

;;; Registration

(jabber-disco-advertise-feature jabber-mam-xmlns)

(jabber-chain-add 'jabber-message-chain #'jabber-mam--process-message -10)
(dolist (chain '(jabber-message-chain jabber-presence-chain jabber-iq-chain))
  (jabber-chain-add chain #'jabber-mam--input-check 100))

(add-hook 'jabber-post-connect-hooks #'jabber-mam-maybe-catchup)
(add-hook 'jabber-pre-disconnect-hook #'jabber-mam--cleanup-all)
(add-hook 'jabber-lifecycle-session-reset-functions #'jabber-mam--cleanup-connection)
(add-hook 'jabber-lost-connection-hooks #'jabber-mam--cleanup-connection)

(provide 'jabber-mam)
;;; jabber-mam.el ends here
