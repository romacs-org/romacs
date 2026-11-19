;;; romacs-agent-service.el --- The agent session protocol, served  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The agents service (doc/agent-protocol.org): the one interface every
;; client of this machine's agent sessions speaks, the local editor's,
;; another machine's (through `romacs-relay') and a phone's.  It listens
;; on `service.sock' under the agents' runtime directory and speaks
;; JSON-RPC lines (romacs-agent-rpc.el).
;;
;; It serves the session list, and one session's live events.
;;
;; The list: `agents/hello', then
;; `agents/sessions.watch', each row as this machine's board shows it
;; (its state key is the board's, made here, never again by a client),
;; replies paged by whole units and continued by `agents/page', and every
;; change after as `agents/sessions.changed'.
;;
;; A row is made again when its session raises an event, with its
;; parent's (whose key follows its workers) and, when it comes, goes or
;; moves, its children's (whose keys follow their parent) and its old
;; parent's (whose count changed); when other processes write its stored
;; row; and every row once a minute, when the editor is idle.
;;
;; A session: `agents/session.open' answers its row and the cursor its
;; live events follow from; each record unit's events after it come as
;; `agents/event', and its row's changes as `agents/session.changed', in
;; one order its revisions number.  Opening reads; it never loads a
;; session, and one not loaded has no live events until it is.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent)
(require 'romacs-agent-rpc)
(require 'romacs-agent-board)
(require 'romacs-agent-wire)
(require 'romacs-agent-events)
(require 'romacs-mcp)

(defconst romacs-agent-service-version '(1 0)
  "The protocol version served: major, minor.")

(defconst romacs-agent-service--unit-budget (* 768 1024)
  "The bytes of units one reply holds at most (Framing).")

(defconst romacs-agent-service--text-limit 1024
  "The most characters of each string in a row: a row stays small, and a
session's full title comes with the session.")

(defconst romacs-agent-service--token-life 60
  "Seconds a continuation token lives unused.")

(defconst romacs-agent-service--tokens-held 4
  "The continuation tokens one connection may hold.")

(defconst romacs-agent-service--whole-every 60
  "Seconds between passes that make every row again, whatever said so.")

(defvar romacs-agent-service--listener nil)
(defvar romacs-agent-service--socket nil)
(defvar romacs-agent-service--socket-identity nil "The listening socket's node, deleted only while it is ours.")
(defvar romacs-agent-service--peers nil "The connections, as `romacs-agent-rpc's.")
(defvar romacs-agent-service--generation nil "This service's generation: a UUID made at its start.")

(cl-defstruct (romacs-agent-service--list (:constructor romacs-agent-service--list-make) (:copier nil))
  "The session list as served: its REVISION; each id's ROW and its JSON
\(TEXT: rows are compared by it, as a table is `equal' only to itself);
the revision each last changed at (SINCE); the ids REMOVED, each with
the revision it went at; and the CHILDREN of each parent id, a table of
their ids."
  (revision 0)
  (rows (make-hash-table :test #'equal))
  (texts (make-hash-table :test #'equal))
  (since (make-hash-table :test #'equal))
  (removed (make-hash-table :test #'equal))
  (children (make-hash-table :test #'equal)))

(defvar romacs-agent-service--list nil "The list as served (`romacs-agent-service--list').")
(defvar romacs-agent-service--sessions (make-hash-table :test #'equal)
  "Each opened session's served state, by id (`romacs-agent-service--served').")
(defvar romacs-agent-service--refresh-timer nil "Makes the rows that may have changed again, soon.")
(defvar romacs-agent-service--poll-timer nil "Looks at the store, prunes tokens, every few seconds.")
(defvar romacs-agent-service--dirty nil "The ids of the sessions whose rows may have changed.")
(defvar romacs-agent-service--store-place nil "Where the store's rows were last looked at (`romacs-agent-rows-since').")
(defvar romacs-agent-service--whole-at 0 "When every row was last made again.")
(defvar romacs-agent-service--unloaded nil
  "The board's view of the unloaded sessions (`romacs-agent-board--unloaded'),
as the last whole pass made it and each row made since adds to it.")

;;; Places and the listener

(defun romacs-agent-service-socket ()
  "Where the service listens: `service.sock' in a directory of its own in
the agents' runtime directory, its user's alone."
  (expand-file-name "service/service.sock" (romacs-agent--runtime-root)))

;;;###autoload
(defun romacs-agent-service-start (&optional socket)
  "Serve the agent session protocol on SOCKET (`romacs-agent-service-socket'
by default), in a directory of this user's alone, replacing only a socket
of this user's that nothing answers at."
  (interactive)
  (when romacs-agent-service--listener (user-error "The agents service already runs"))
  (let ((socket (or socket (romacs-agent-service-socket))))
    (server-ensure-safe-dir (file-name-directory socket))
    (when (or (file-exists-p socket) (file-symlink-p socket))
      (let ((stale (romacs-mcp--stale-socket-id socket)))
        (unless stale (user-error "%s is there and is no stale socket of yours" socket))
        (unless (equal stale (romacs-mcp--owned-socket-snapshot socket))
          (user-error "%s changed while the service started" socket))
        (let (delete-by-moving-to-trash) (delete-file socket))))
    (condition-case err
        (progn
          (setq romacs-agent-service--generation (romacs-agent--uuid)
                romacs-agent-service--list (romacs-agent-service--list-make)
                romacs-agent-service--socket socket
                romacs-agent-service--listener
                (make-network-process :name "romacs-agent-service" :family 'local :service socket
                                      :server t :noquery t :coding 'no-conversion
                                      :filter #'romacs-agent-service--filter
                                      :sentinel #'romacs-agent-service--sentinel
                                      :log #'romacs-agent-service--accept)
                romacs-agent-service--socket-identity (romacs-mcp--socket-id socket))
          (setq romacs-agent-service--store-place
                (and romacs-agent-rows-since (cdr (funcall romacs-agent-rows-since nil))))
          (romacs-agent-service--refresh t)
          (add-hook 'romacs-agent-event-functions #'romacs-agent-service--event)
          (setq romacs-agent-service--poll-timer (run-at-time 3 3 #'romacs-agent-service--poll))
          (add-hook 'kill-emacs-hook #'romacs-agent-service-stop))
      (error (romacs-agent-service-stop) (signal (car err) (cdr err))))
    socket))

(defun romacs-agent-service-stop ()
  "Stop serving: close every connection, and the socket while it is ours."
  (interactive)
  (remove-hook 'romacs-agent-event-functions #'romacs-agent-service--event)
  (remove-hook 'kill-emacs-hook #'romacs-agent-service-stop)
  (dolist (timer (list romacs-agent-service--refresh-timer romacs-agent-service--poll-timer
                       romacs-agent-service--idle-timer))
    (when (timerp timer) (cancel-timer timer)))
  (mapc #'romacs-agent-rpc-close romacs-agent-service--peers)
  (when (process-live-p romacs-agent-service--listener) (delete-process romacs-agent-service--listener))
  (when (and romacs-agent-service--socket romacs-agent-service--socket-identity
             (equal (romacs-mcp--socket-id romacs-agent-service--socket) romacs-agent-service--socket-identity))
    (let (delete-by-moving-to-trash) (delete-file romacs-agent-service--socket)))
  (setq romacs-agent-service--listener nil romacs-agent-service--socket nil romacs-agent-service--socket-identity nil
        romacs-agent-service--peers nil romacs-agent-service--refresh-timer nil romacs-agent-service--poll-timer nil
        romacs-agent-service--list nil romacs-agent-service--dirty nil romacs-agent-service--unloaded nil
        romacs-agent-service--whole-at 0 romacs-agent-service--idle-timer nil)
  (clrhash romacs-agent-service--sessions))

;;; Connections

(cl-defstruct (romacs-agent-service--peer (:constructor romacs-agent-service--peer-make) (:copier nil))
  "What the service keeps of a connection: whether it said hello, its
identity, whether it watches the list, its continuation tokens, each
\(TOKEN TIME . STATE), and the ids of the sessions it opened (OPEN, a
table)."
  hello identity watching tokens (open (make-hash-table :test #'equal)))

(defun romacs-agent-service--accept (_listener process _message)
  (let ((rpc (romacs-agent-rpc-make process (romacs-agent-service--peer-make
                                            :identity (romacs-agent-rpc-object "via" "local"
                                                                               "login" (user-login-name))))))
    (process-put process 'romacs-agent-rpc rpc)
    (push rpc romacs-agent-service--peers)))

(defun romacs-agent-service--sentinel (process _event)
  (unless (process-live-p process)
    (let ((rpc (process-get process 'romacs-agent-rpc)))
      (setq romacs-agent-service--peers (delq rpc romacs-agent-service--peers))
      ;; A session no connection has open any more keeps nothing.
      (when rpc
        (maphash (lambda (id _) (unless (romacs-agent-service--subscribers id) (remhash id romacs-agent-service--sessions)))
                 (romacs-agent-service--peer-open (romacs-agent-rpc-data rpc)))))))

(defun romacs-agent-service--filter (process chunk)
  (when-let* ((rpc (process-get process 'romacs-agent-rpc)))
    (let ((lines (romacs-agent-rpc-lines rpc chunk)))
      (if (eq lines 'over)
          (romacs-agent-rpc-close rpc)
        (dolist (line lines) (romacs-agent-service--handle rpc line))))))

(defconst romacs-agent-service--methods
  '(("agents/hello" . romacs-agent-service--hello)
    ("agents/sessions.watch" . romacs-agent-service--watch)
    ("agents/page" . romacs-agent-service--page)
    ("agents/session.open" . romacs-agent-service--open)
    ("agents/session.close" . romacs-agent-service--close))
  "Each method served and its function, called with the connection and
the params; it returns the result, or signals `romacs-agent-service-error'.")

(define-error 'romacs-agent-service-error "Agent protocol error")

(defun romacs-agent-service--fail (code kind message &optional data)
  (signal 'romacs-agent-service-error (list code kind message data)))

(defun romacs-agent-service--answer (rpc id method params)
  "The reply to the request ID of METHOD with PARAMS on RPC."
  (condition-case err
      (let ((function (cdr (assoc method romacs-agent-service--methods))))
        (cond ((and (not (romacs-agent-service--peer-hello (romacs-agent-rpc-data rpc)))
                    (not (equal method "agents/hello")))
               (romacs-agent-service--fail -32002 "not-initialized" "Say agents/hello first"))
              ((null function) (romacs-agent-service--fail -32601 "unknown-method" "No such method"))
              ((not (hash-table-p params)) (romacs-agent-service--fail -32602 "bad-params" "Params must be an object")))
        (romacs-agent-rpc-result id (funcall function rpc params)))
    (romacs-agent-service-error
     (pcase-let ((`(,code ,kind ,text ,data) (cdr err))) (romacs-agent-rpc-error id code kind text data)))
    (error (romacs-agent-rpc-error id -32603 "internal" (error-message-string err)))))

(defun romacs-agent-service--handle (rpc line)
  "Answer the message LINE on RPC: a request is answered; a notification
from a client, of which none is defined, is ignored."
  (let* ((message (romacs-agent-rpc-parse line))
         (id (and message (gethash "id" message :none)))
         (method (and message (gethash "method" message))))
    (cond
     ((null message)
      (romacs-agent-rpc-send rpc (romacs-agent-rpc-error nil -32700 "parse-error" "Not a JSON object")))
     ((eq id :none))
     ;; Its id echoed only when it may be (Framing): a string or a number,
     ;; 128 bytes at most.
     ((or (not (or (stringp id) (numberp id))) (> (string-bytes (json-serialize id)) 128))
      (romacs-agent-rpc-send rpc (romacs-agent-rpc-error nil -32600 "bad-request" "A request id is a string or number of 128 bytes at most")))
     ((or (not (equal (gethash "jsonrpc" message) "2.0")) (not (stringp method)))
      (romacs-agent-rpc-send rpc (romacs-agent-rpc-error id -32600 "bad-request" "Not a JSON-RPC 2.0 request")))
     (t (romacs-agent-rpc-send rpc (romacs-agent-service--answer rpc id method
                                                                  (gethash "params" message (make-hash-table :test #'equal))))))))

(defun romacs-agent-service--hello (rpc params)
  "`agents/hello': the version both speak, this service, the caller's identity."
  (let* ((version (gethash "version" params))
         (major (and (vectorp version) (> (length version) 1) (aref version 0)))
         (minor (and major (aref version 1))))
    (unless (and (equal (gethash "protocol" params) "romacs-agents") (integerp major) (integerp minor))
      (romacs-agent-service--fail -32602 "bad-params" "Say protocol romacs-agents and version [MAJOR, MINOR]"))
    (unless (= major (car romacs-agent-service-version))
      (romacs-agent-service--fail -32013 "unsupported" "Another major version"
                                  (romacs-agent-rpc-object "served" (vconcat romacs-agent-service-version))))
    (let ((peer (romacs-agent-rpc-data rpc)))
      (setf (romacs-agent-service--peer-hello peer) t)
      (romacs-agent-rpc-object
       "version" (vector major (min minor (cadr romacs-agent-service-version)))
       "server" (romacs-agent-rpc-object "host" (system-name) "build" emacs-version
                                         "generation" romacs-agent-service--generation)
       "identity" (romacs-agent-service--peer-identity peer)
       "capabilities" []))))

;;; Rows

(defvar romacs-agent-service--projects (make-hash-table :test #'equal)
  "Each working directory's project name (`romacs-agent-board--project'),
made once: a name is abbreviated through the file system, and a list's
rows share few directories.")

(defun romacs-agent-service--project (cwd)
  (or (gethash cwd romacs-agent-service--projects)
      (puthash cwd (romacs-agent-board--project cwd) romacs-agent-service--projects)))

(defun romacs-agent-service--number (value)
  (if (numberp value) value :null))

(defun romacs-agent-service--text (value)
  "VALUE, a string cut to `romacs-agent-service--text-limit' characters, or null."
  (if (stringp value) (substring value 0 (min (length value) romacs-agent-service--text-limit)) :null))

(defun romacs-agent-service-row (session key)
  "SESSION as a row (Protocol v1, The session list), KEY its state key on
the board; its children are counted where the rows are kept."
  (let ((options (romacs-agent-options session)))
    (romacs-agent-rpc-object
     "id" (romacs-agent-id session)
     "title" (romacs-agent-service--text (romacs-agent-title session))
     "cwd" (romacs-agent-service--text (romacs-agent-cwd session))
     "project" (romacs-agent-service--text (romacs-agent-service--project (romacs-agent-cwd session)))
     "parent" (romacs-agent-service--text (plist-get options :parent))
     "harness" (symbol-name (romacs-agent-harness session))
     "provider" (romacs-agent-service--text (romacs-agent-provider session))
     "model" (romacs-agent-service--text (romacs-agent-model session))
     "effort" (romacs-agent-service--text (plist-get options :effort))
     "state" (symbol-name (romacs-agent-state session))
     "key" (symbol-name key)
     "pending" (let ((approvals (romacs-agent-approvals session)))
                 (if (hash-table-p approvals) (hash-table-count approvals) 0))
     "queued" (or (romacs-agent-queue-length session) 0)
     "last_time" (romacs-agent-service--number (romacs-agent-last-time session))
     "cost" (romacs-agent-service--number (romacs-agent-cost session))
     "context_used" (romacs-agent-service--number (romacs-agent-context-used session))
     "context_window" (romacs-agent-service--number (romacs-agent-context-window session))
     "children" 0)))

(defun romacs-agent-service--whole-rows ()
  "Every session this machine's board shows, as rows by id, their keys the
board's own (`romacs-agent-board--key'), from the board's own sessions
\(`romacs-agent-board--sessions').  Its view of the unloaded ones is the
service's own, kept for the rows made before the next whole pass: the
board's stays the board's, as the rows it shows hold its sessions."
  (let ((romacs-agent-board--unloaded nil))
    (let* ((sessions (romacs-agent-board--sessions))
           (rows (make-hash-table :test #'equal :size (length sessions))))
      (setq romacs-agent-service--unloaded romacs-agent-board--unloaded)
      (dolist (session sessions)
        (let ((id (romacs-agent-id session)))
          (unless (gethash id rows)
            (puthash id (romacs-agent-service-row session (romacs-agent-board--key session)) rows))))
      rows)))

(defun romacs-agent-service--session (id)
  "Session ID as it is now, loaded or from its row, or nil when it is gone;
the board's view of the unloaded sessions (`romacs-agent-service--unloaded')
made to agree, as a child's key reads its parent there."
  (let ((session (or (romacs-agent-loaded id) (romacs-agent-store-row-session id)))
        (unloaded (car romacs-agent-service--unloaded)))
    (when unloaded
      (if (and session (not (romacs-agent-loaded id))) (puthash id session unloaded) (remhash id unloaded)))
    session))

(defun romacs-agent-service--parent (row)
  (let ((parent (and row (gethash "parent" row)))) (and (stringp parent) parent)))

(defun romacs-agent-service--rows-of (list ids)
  "The rows of IDS made again, as updates (ID . ROW) of LIST, ROW nil for a
session gone; with each one's parent, whose key follows its workers, and,
for one that came, went or moved, its children, whose keys follow their
parent, and its old parent, whose count changed.  Every session of the
batch is looked up before any key is made, as a key reads others."
  (let ((rows (romacs-agent-service--list-rows list))
        (children (romacs-agent-service--list-children list))
        (sessions (make-hash-table :test #'equal)) (order nil) (todo ids))
    (while todo
      (let ((id (pop todo)))
        (unless (gethash id sessions)
          (let* ((session (romacs-agent-service--session id))
                 (parent (and session (plist-get (romacs-agent-options session) :parent)))
                 (was (gethash id rows)) (old (romacs-agent-service--parent was)))
            (puthash id (or session :gone) sessions)
            (push id order)
            (when (stringp parent) (push parent todo))
            (unless (and session was (equal parent old))
              (when old (push old todo))
              (when-let* ((kids (gethash id children)))
                (maphash (lambda (kid _) (push kid todo)) kids)))))))
    (let ((romacs-agent-board--unloaded romacs-agent-service--unloaded))
      (mapcar (lambda (id)
                (let ((session (gethash id sessions)))
                  (cons id (and (not (eq session :gone))
                                (romacs-agent-service-row session (romacs-agent-board--key session))))))
              (nreverse order)))))

(defun romacs-agent-service--apply (list updates)
  "Apply UPDATES, each (ID . ROW), to LIST: the children index kept, each
row's children counted from it, and the rows whose text changed kept
under a new revision.  (REVISION CHANGED REMOVED), or nil when nothing
changed."
  (let ((rows (romacs-agent-service--list-rows list)) (texts (romacs-agent-service--list-texts list))
        (children (romacs-agent-service--list-children list))
        (changed nil) (removed nil))
    ;; The index first: a row's count reads it.
    (pcase-dolist (`(,id . ,row) updates)
      (let ((old (romacs-agent-service--parent (gethash id rows))) (new (romacs-agent-service--parent row)))
        (unless (equal old new)
          (when-let* ((kids (and old (gethash old children))))
            (remhash id kids)
            (when (zerop (hash-table-count kids)) (remhash old children)))
          (when new
            (puthash id t (or (gethash new children)
                              (puthash new (make-hash-table :test #'equal) children)))))))
    (pcase-dolist (`(,id . ,row) updates)
      (if (null row)
          (when (gethash id rows) (push id removed))
        (puthash "children" (let ((kids (gethash id children))) (if kids (hash-table-count kids) 0)) row)
        (let ((text (json-serialize row)))
          (unless (equal text (gethash id texts))
            (puthash id text texts)
            (push row changed)))))
    (when (or changed removed)
      (let ((revision (cl-incf (romacs-agent-service--list-revision list))))
        (dolist (row changed)
          (let ((id (gethash "id" row)))
            (puthash id row rows)
            (puthash id revision (romacs-agent-service--list-since list))
            (remhash id (romacs-agent-service--list-removed list))))
        (dolist (id removed)
          (remhash id rows)
          (remhash id texts)
          (remhash id (romacs-agent-service--list-since list))
          (puthash id revision (romacs-agent-service--list-removed list)))
        (list revision changed removed)))))

(defun romacs-agent-service--whole-updates (list)
  "Every row made again, as updates of LIST: each session's, and each id
LIST serves that no session has."
  (let ((rows (romacs-agent-service--whole-rows)) (updates nil))
    (maphash (lambda (id row) (push (cons id row) updates)) rows)
    (maphash (lambda (id _) (unless (gethash id rows) (push (cons id nil) updates)))
             (romacs-agent-service--list-rows list))
    updates))

(defun romacs-agent-service--refresh (&optional whole)
  "Make the rows that may have changed again (every row when WHOLE), and
tell the watchers what changed, and each session's subscribers its row."
  (setq romacs-agent-service--refresh-timer nil)
  (when romacs-agent-service--list
    (let* ((list romacs-agent-service--list)
           (whole (and whole t))
           (dirty (prog1 romacs-agent-service--dirty (setq romacs-agent-service--dirty nil)))
           (updates (if whole
                        (progn (setq romacs-agent-service--whole-at (float-time))
                               (romacs-agent-service--whole-updates list))
                      (romacs-agent-service--rows-of list (delete-dups dirty)))))
      (pcase-let ((`(,revision ,changed ,removed) (romacs-agent-service--apply list updates)))
        (when revision
          (romacs-agent-service--tell revision changed removed)
          (dolist (row changed)
            (romacs-agent-service--to-subscribers
             (gethash "id" row) "agents/session.changed" (lambda (params) (puthash "row" row params)))))))))

(defun romacs-agent-service--tell (revision changed removed)
  "Tell the watchers the list's REVISION: the rows CHANGED, the ids REMOVED;
too big for one message, that it is stale, and they ask for what changed."
  (let ((message (romacs-agent-rpc-notification
                  "agents/sessions.changed"
                  (romacs-agent-rpc-object "revision" revision "rows" (vconcat changed)
                                           "removed" (vconcat removed)))))
    (when (> (string-bytes (json-serialize message)) romacs-agent-rpc-message-limit)
      (setq message (romacs-agent-rpc-notification
                     "agents/sessions.changed" (romacs-agent-rpc-object "revision" revision "stale" t))))
    (dolist (rpc romacs-agent-service--peers)
      (when (romacs-agent-service--peer-watching (romacs-agent-rpc-data rpc))
        (romacs-agent-rpc-send rpc message)))))

(defun romacs-agent-service--refresh-soon ()
  (unless romacs-agent-service--refresh-timer
    (setq romacs-agent-service--refresh-timer (run-at-time 0.1 nil #'romacs-agent-service--refresh))))

(defun romacs-agent-service--event (session event)
  "SESSION raised EVENT: its row may have changed, and its subscribers are
sent its record unit once it is whole.  Never signals: it runs in the
session's fold."
  (condition-case err
      (let ((id (romacs-agent-id session)))
        (push id romacs-agent-service--dirty)
        (romacs-agent-service--refresh-soon)
        (romacs-agent-service--unit-event id session event))
    (error (message "romacs-agent-service: %s" (error-message-string err)))))

(defvar romacs-agent-service--idle-timer nil "Runs the whole pass once the editor is idle.")

(defun romacs-agent-service--whole-when-idle ()
  "Make every row again once the editor is idle: a pass over every row
takes tens of milliseconds, never while the user types."
  (unless (timerp romacs-agent-service--idle-timer)
    (setq romacs-agent-service--idle-timer
          (run-with-idle-timer 2 nil (lambda ()
                                       (setq romacs-agent-service--idle-timer nil)
                                       (romacs-agent-service--refresh t))))))

(defun romacs-agent-service--poll ()
  "The rows other processes wrote since the last look may have changed;
expired tokens are let go."
  (when romacs-agent-rows-since
    (pcase-let ((`(,ids . ,place) (funcall romacs-agent-rows-since romacs-agent-service--store-place)))
      (setq romacs-agent-service--store-place place
            romacs-agent-service--dirty (append ids romacs-agent-service--dirty))))
  (dolist (rpc romacs-agent-service--peers)
    (let ((peer (romacs-agent-rpc-data rpc)))
      (setf (romacs-agent-service--peer-tokens peer)
            (cl-remove-if (lambda (entry) (> (- (float-time) (cadr entry)) romacs-agent-service--token-life))
                          (romacs-agent-service--peer-tokens peer)))))
  (when (> (- (float-time) romacs-agent-service--whole-at) romacs-agent-service--whole-every)
    (romacs-agent-service--whole-when-idle))
  (when romacs-agent-service--dirty (romacs-agent-service--refresh-soon)))

;;; Watching, paging

(defun romacs-agent-service--watch (rpc params)
  "`agents/sessions.watch': every row, or, with SINCE a revision of this
generation (GENERATION names the one it is of), those changed after it
and the ids removed after it."
  (let* ((list romacs-agent-service--list)
         (since (gethash "since" params))
         (since (and (integerp since) (equal (gethash "generation" params) romacs-agent-service--generation)
                     (<= since (romacs-agent-service--list-revision list))
                     since))
         (rows nil) (removed nil))
    (maphash (lambda (id row)
               (when (or (null since) (> (gethash id (romacs-agent-service--list-since list) 0) since))
                 (push row rows)))
             (romacs-agent-service--list-rows list))
    (when since
      (maphash (lambda (id revision) (when (> revision since) (push id removed)))
               (romacs-agent-service--list-removed list)))
    (setf (romacs-agent-service--peer-watching (romacs-agent-rpc-data rpc)) t)
    (romacs-agent-service--paged rpc (list :revision (romacs-agent-service--list-revision list)
                                           :reset (null since) :rows rows :removed removed))))

(defun romacs-agent-service--paged (rpc state)
  "STATE, a list reply's (:revision :reset :rows :removed), as one reply:
whole units (rows, then removed ids) up to the budget, at least one, and
a token continuing it."
  (let ((budget romacs-agent-service--unit-budget)
        (rows (plist-get state :rows)) (removed (plist-get state :removed))
        (taken-rows nil) (taken-removed nil) (first t))
    (cl-flet ((fits (unit) (let ((bytes (string-bytes (json-serialize unit))))
                             (when (or first (<= bytes budget))
                               (setq budget (- budget bytes) first nil)
                               t))))
      (while (and rows (fits (car rows))) (push (pop rows) taken-rows))
      (unless rows
        (while (and removed (fits (car removed))) (push (pop removed) taken-removed))))
    (romacs-agent-rpc-object
     "revision" (plist-get state :revision) "reset" (if (plist-get state :reset) t :false)
     "rows" (vconcat (nreverse taken-rows))
     "removed" (vconcat (nreverse taken-removed))
     ;; The rest continues this reply: no reset again.
     "more" (if (or rows removed)
                (romacs-agent-service--token rpc (list :revision (plist-get state :revision) :reset nil
                                                       :rows rows :removed removed))
              :null))))

(defun romacs-agent-service--token (rpc state)
  "A continuation token of RPC's for STATE; its oldest token dies when it
holds too many."
  (let* ((peer (romacs-agent-rpc-data rpc))
         (token (substring (romacs-agent--uuid) 0 18))
         (tokens (cons (cons token (cons (float-time) state)) (romacs-agent-service--peer-tokens peer))))
    (setf (romacs-agent-service--peer-tokens peer) (seq-take tokens romacs-agent-service--tokens-held))
    token))

(defun romacs-agent-service--page (rpc params)
  "`agents/page': the rest of a paged reply, by its token."
  (let* ((peer (romacs-agent-rpc-data rpc))
         (token (gethash "more" params))
         (entry (assoc token (romacs-agent-service--peer-tokens peer))))
    (setf (romacs-agent-service--peer-tokens peer) (delq entry (romacs-agent-service--peer-tokens peer)))
    (if (or (null entry) (> (- (float-time) (cadr entry)) romacs-agent-service--token-life))
        (romacs-agent-service--fail -32017 "stale" "The token expired: start the call again")
      (romacs-agent-service--paged rpc (cddr entry)))))

;;; One session

(defconst romacs-agent-service--string-limit (* 64 1024)
  "The most bytes of one string in an event as sent: the rest is cut, the
event saying how long it was (Framing).")

(defconst romacs-agent-service--unit-limit (* 256 1024)
  "The most bytes of one record unit's events as sent.")

(cl-defstruct (romacs-agent-service--served (:constructor romacs-agent-service--served-make) (:copier nil))
  "What the service keeps of an opened session: its REVISION, which orders
everything sent of it, and the EVENTS of the record unit under way (last
first)."
  (revision 0) events)

(defun romacs-agent-service--served (id)
  (or (gethash id romacs-agent-service--sessions)
      (puthash id (romacs-agent-service--served-make) romacs-agent-service--sessions)))

(defun romacs-agent-service--subscribers (id)
  "The connections that opened session ID."
  (seq-filter (lambda (rpc) (gethash id (romacs-agent-service--peer-open (romacs-agent-rpc-data rpc))))
              romacs-agent-service--peers))

(defun romacs-agent-service--to-subscribers (id method fill)
  "Send session ID's subscribers METHOD under its next revision, its params
the session and the revision, then what FILL puts in them."
  (when-let* ((subscribers (romacs-agent-service--subscribers id)))
    (let* ((served (romacs-agent-service--served id))
           (params (romacs-agent-rpc-object "session" id
                                            "revision" (cl-incf (romacs-agent-service--served-revision served)))))
      (funcall fill params)
      (let ((message (romacs-agent-rpc-notification method params)))
        (dolist (rpc subscribers) (romacs-agent-rpc-send rpc message))))))

(defvar romacs-agent-service--adapters (make-hash-table :test #'eq)
  "Each harness's adapter version: a digest of its driver's source and of
the event schema, made once.")

(defun romacs-agent-service--adapter (harness)
  "The adapter version of HARNESS: what made a session's events, so a
client's events of another are not resumed (Cursors)."
  (or (gethash harness romacs-agent-service--adapters)
      (puthash harness
               (let ((file (locate-library (format "romacs-agent-%s" harness) t)))
                 (substring (secure-hash 'sha256 (concat (prin1-to-string romacs-agent-wire-events)
                                                         (if (and file (file-readable-p file))
                                                             (with-temp-buffer (insert-file-contents file) (buffer-string))
                                                           "")))
                            0 16))
               romacs-agent-service--adapters)))

(defun romacs-agent-service--cursor (session seq)
  "The cursor at SEQ of SESSION's records."
  (romacs-agent-rpc-object "seq" seq "epoch" (romacs-agent-events-epoch (romacs-agent-id session))
                           "adapter" (romacs-agent-service--adapter (romacs-agent-harness session))))

(defun romacs-agent-service--cut (value limit cut path)
  "VALUE, a sent event's or a part of one, with each string over LIMIT
bytes cut to its first LIMIT; each cut's PATH and length pushed to CUT,
a cons whose car holds them."
  (cond ((and (stringp value) (> (string-bytes value) limit))
         (push (cons path (string-bytes value)) (car cut))
         (let ((end (min (length value) limit)))
           (while (> (string-bytes (substring value 0 end)) limit) (setq end (- end (max 1 (/ (- (string-bytes (substring value 0 end)) limit) 4)))))
           (substring value 0 end)))
        ((hash-table-p value)
         (let ((copy (make-hash-table :test #'equal :size (hash-table-count value))))
           (maphash (lambda (key item)
                      (puthash key (romacs-agent-service--cut item limit cut (if path (concat path "." key) key)) copy))
                    value)
           copy))
        ((vectorp value)
         (let ((i -1))
           (vconcat (mapcar (lambda (item)
                              (romacs-agent-service--cut item limit cut (format "%s[%d]" (or path "") (cl-incf i))))
                            value))))
        (t value)))

(defun romacs-agent-service--bounded (events)
  "EVENTS as sent, a vector of wire objects, within the unit's limit: each
string cut to the string limit, then to less while the unit is over, an
event with a cut string saying each one's length under `truncated'.  A
unit still over is (oversize): its events are not sent."
  (let ((unit (romacs-agent-wire-unit events)))
    (or (cl-loop for limit in (list romacs-agent-service--string-limit 8192 1024)
                 for bounded = (vconcat (mapcar (lambda (event)
                                                  (let* ((cut (list nil))
                                                         (sent (romacs-agent-service--cut event limit cut nil)))
                                                    (when (car cut)
                                                      (puthash "truncated" (let ((lengths (make-hash-table :test #'equal)))
                                                                             (pcase-dolist (`(,path . ,bytes) (car cut))
                                                                               (puthash path bytes lengths))
                                                                             lengths)
                                                               sent))
                                                    sent))
                                                unit))
                 when (<= (string-bytes (json-serialize bounded)) romacs-agent-service--unit-limit)
                 return bounded)
        'oversize)))

(defun romacs-agent-service--unit-event (id session event)
  "Keep SESSION's EVENT for its record unit, sent to its subscribers once
the unit is whole (its last event).  Only the session the editor has
loaded: a replay's copy shares its id, and is no live event."
  (when (and (gethash id romacs-agent-service--sessions)
             (eq session (romacs-agent-loaded id))
             (plist-get event :seq))
    (let ((served (romacs-agent-service--served id)))
      (push event (romacs-agent-service--served-events served))
      (when (plist-get event :last)
        (let* ((events (nreverse (romacs-agent-service--served-events served)))
               (seq (plist-get event :seq))
               (sent (romacs-agent-service--bounded events)))
          (setf (romacs-agent-service--served-events served) nil)
          (unless (equal sent [])
            (romacs-agent-service--to-subscribers
             id "agents/event"
             (lambda (params)
               (puthash "cursor" (romacs-agent-service--cursor session seq) params)
               (if (eq sent 'oversize)
                   (progn (puthash "events" [] params) (puthash "oversize" t params))
                 (puthash "events" sent params))))))))))

(defun romacs-agent-service--open (rpc params)
  "`agents/session.open': session ID's row, its revision, and the cursor its
live events follow from; its subscription made.  It reads: a session not
loaded stays so."
  (let* ((id (gethash "id" params))
         (session (and (stringp id) (or (romacs-agent-loaded id) (romacs-agent-store-row-session id))))
         (row (and session (gethash id (romacs-agent-service--list-rows romacs-agent-service--list)))))
    (unless session (romacs-agent-service--fail -32010 "unknown-session" "No such session"))
    (puthash id t (romacs-agent-service--peer-open (romacs-agent-rpc-data rpc)))
    (let ((served (romacs-agent-service--served id)))
      (romacs-agent-rpc-object
       "revision" (romacs-agent-service--served-revision served)
       "cursor" (romacs-agent-service--cursor
                 session (if (romacs-agent-loaded id) (romacs-agent-seq session)
                           (1- (romacs-agent-events-end session))))
       "row" (or row :null)
       "pending" [] "answering" [] "queue" [] "undelivered" [] "more" :null))))

(defun romacs-agent-service--close (rpc params)
  "`agents/session.close': session ID's subscription let go."
  (let ((id (gethash "id" params)))
    (remhash id (romacs-agent-service--peer-open (romacs-agent-rpc-data rpc)))
    (unless (romacs-agent-service--subscribers id) (remhash id romacs-agent-service--sessions))
    (romacs-agent-rpc-object)))


(provide 'romacs-agent-service)
;;; romacs-agent-service.el ends here
