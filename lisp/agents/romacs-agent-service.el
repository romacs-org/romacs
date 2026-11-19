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
;; This part serves the session list: `agents/hello', then
;; `agents/sessions.watch', each row as this machine's board shows it
;; (its state key is the board's, made here, never again by a client),
;; paged replies continued by `agents/page', and every change after as
;; `agents/sessions.changed'.  Rows are made again a moment after any
;; session raises an event, and every few seconds for rows other
;; processes write.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent)
(require 'romacs-agent-rpc)
(require 'romacs-agent-board)

(defconst romacs-agent-service-version '(1 0)
  "The protocol version served: major, minor.")

(defconst romacs-agent-service--unit-budget (* 768 1024)
  "The bytes of units one reply holds at most (Framing).")

(defconst romacs-agent-service--token-life 60
  "Seconds a continuation token lives unused.")

(defconst romacs-agent-service--tokens-held 4
  "The continuation tokens one connection may hold.")

(defvar romacs-agent-service--listener nil)
(defvar romacs-agent-service--socket nil)
(defvar romacs-agent-service--peers nil "The connections, as `romacs-agent-rpc's.")
(defvar romacs-agent-service--generation nil "This service's generation: a UUID made at its start.")

(cl-defstruct (romacs-agent-service--list (:constructor romacs-agent-service--list-make) (:copier nil))
  "The session list as served: its REVISION, each id's ROW and its JSON
\(TEXT: rows are compared by it, as a table is `equal' only to itself),
the revision each last changed at (SINCE), and the ids REMOVED, each
with the revision it went at."
  (revision 0)
  (rows (make-hash-table :test #'equal))
  (texts (make-hash-table :test #'equal))
  (since (make-hash-table :test #'equal))
  (removed (make-hash-table :test #'equal)))

(defvar romacs-agent-service--list nil "The list as served (`romacs-agent-service--list').")
(defvar romacs-agent-service--refresh-timer nil "Makes the rows again, once, soon.")
(defvar romacs-agent-service--poll-timer nil "Makes the rows again every few seconds.")

;;; Places and the listener

(defun romacs-agent-service-socket ()
  "Where the service listens: `service.sock' in the agents' runtime directory."
  (expand-file-name "service.sock" (romacs-agent--runtime-root)))

(defun romacs-agent-service--uuid ()
  (let ((hex (secure-hash 'sha256 (format "%s%s%s" (random) (float-time) (emacs-pid)))))
    (format "%s-%s-4%s-%s-%s" (substring hex 0 8) (substring hex 8 12) (substring hex 13 16)
            (substring hex 16 20) (substring hex 20 32))))

;;;###autoload
(defun romacs-agent-service-start (&optional socket)
  "Serve the agent session protocol on SOCKET (`romacs-agent-service-socket'
by default), replacing a socket nothing answers at."
  (interactive)
  (when romacs-agent-service--listener (user-error "The agents service already runs"))
  (let ((socket (or socket (romacs-agent-service-socket))))
    (make-directory (file-name-directory socket) t)
    (set-file-modes (file-name-directory socket) #o700)
    (when (file-exists-p socket)
      (if (romacs-agent-service--answers-p socket)
          (user-error "An agents service already answers at %s" socket)
        (let (delete-by-moving-to-trash) (delete-file socket))))
    (setq romacs-agent-service--generation (romacs-agent-service--uuid)
          romacs-agent-service--list (romacs-agent-service--list-make)
          romacs-agent-service--socket socket
          romacs-agent-service--listener
          (make-network-process :name "romacs-agent-service" :family 'local :service socket
                                :server t :noquery t :coding 'no-conversion
                                :filter #'romacs-agent-service--filter
                                :sentinel #'romacs-agent-service--sentinel
                                :log #'romacs-agent-service--accept))
    (setq romacs-agent-service--store-place
          (and romacs-agent-rows-since (cdr (funcall romacs-agent-rows-since nil))))
    (romacs-agent-service--refresh t)
    (add-hook 'romacs-agent-event-functions #'romacs-agent-service--event)
    (setq romacs-agent-service--poll-timer (run-at-time 3 3 #'romacs-agent-service--poll))
    (add-hook 'kill-emacs-hook #'romacs-agent-service-stop)
    socket))

(defun romacs-agent-service--answers-p (socket)
  (condition-case nil
      (progn (delete-process (make-network-process :name "romacs-agent-service-probe" :family 'local
                                                   :service socket :noquery t))
             t)
    (error nil)))

(defun romacs-agent-service-stop ()
  "Stop serving: close every connection and the socket."
  (interactive)
  (remove-hook 'romacs-agent-event-functions #'romacs-agent-service--event)
  (remove-hook 'kill-emacs-hook #'romacs-agent-service-stop)
  (dolist (timer (list romacs-agent-service--refresh-timer romacs-agent-service--poll-timer))
    (when (timerp timer) (cancel-timer timer)))
  (mapc #'romacs-agent-rpc-close romacs-agent-service--peers)
  (when (process-live-p romacs-agent-service--listener) (delete-process romacs-agent-service--listener))
  (when (and romacs-agent-service--socket (file-exists-p romacs-agent-service--socket))
    (let (delete-by-moving-to-trash) (delete-file romacs-agent-service--socket)))
  (setq romacs-agent-service--listener nil romacs-agent-service--socket nil romacs-agent-service--peers nil
        romacs-agent-service--refresh-timer nil romacs-agent-service--poll-timer nil))

;;; Connections

(cl-defstruct (romacs-agent-service--peer (:constructor romacs-agent-service--peer-make) (:copier nil))
  "What the service keeps of a connection: whether it said hello, its
identity, whether it watches the list, and its continuation tokens, each
\(TOKEN . (TIME . STATE))."
  hello identity watching tokens)

(defun romacs-agent-service--accept (_listener process _message)
  (let ((rpc (romacs-agent-rpc-make process (romacs-agent-service--peer-make
                                            :identity (romacs-agent-rpc-object "via" "local"
                                                                               "login" (user-login-name))))))
    (process-put process 'romacs-agent-rpc rpc)
    (push rpc romacs-agent-service--peers)))

(defun romacs-agent-service--sentinel (process _event)
  (unless (process-live-p process)
    (setq romacs-agent-service--peers (delq (process-get process 'romacs-agent-rpc) romacs-agent-service--peers))))

(defun romacs-agent-service--filter (process chunk)
  (when-let* ((rpc (process-get process 'romacs-agent-rpc)))
    (dolist (line (romacs-agent-rpc-lines rpc chunk))
      (romacs-agent-service--handle rpc line))))

(defconst romacs-agent-service--methods
  '(("agents/hello" . romacs-agent-service--hello)
    ("agents/sessions.watch" . romacs-agent-service--watch)
    ("agents/page" . romacs-agent-service--page))
  "Each method served and its function, called with the connection and
the params; it returns the result, or signals `romacs-agent-service-error'.")

(define-error 'romacs-agent-service-error "Agent protocol error")

(defun romacs-agent-service--fail (code kind message &optional data)
  (signal 'romacs-agent-service-error (list code kind message data)))

(defun romacs-agent-service--handle (rpc line)
  "Answer the message LINE on RPC."
  (let* ((message (romacs-agent-rpc-parse line))
         (id (and message (gethash "id" message)))
         (method (and message (gethash "method" message))))
    (cond
     ((null message)
      (romacs-agent-rpc-send rpc (romacs-agent-rpc-error nil -32700 "parse-error" "Not a JSON object")))
     ;; Its id echoed only when it is short (Framing).
     ((> (string-bytes (json-serialize (or id :null))) 128)
      (romacs-agent-rpc-send rpc (romacs-agent-rpc-error nil -32600 "bad-request" "Request id over 128 bytes")))
     ((not (stringp method)))
     (t
      (let ((reply
             (condition-case err
                 (let ((function (cdr (assoc method romacs-agent-service--methods))))
                   (cond ((null function) (romacs-agent-service--fail -32601 "unknown-method" (format "No method %s" method)))
                         ((and (not (romacs-agent-service--peer-hello (romacs-agent-rpc-data rpc)))
                               (not (equal method "agents/hello")))
                          (romacs-agent-service--fail -32002 "not-initialized" "Say agents/hello first")))
                   (romacs-agent-rpc-result id (funcall function rpc (let ((params (gethash "params" message)))
                                                                    (if (hash-table-p params) params
                                                                      (make-hash-table :test #'equal))))))
               (romacs-agent-service-error
                (pcase-let ((`(,code ,kind ,text ,data) (cdr err)))
                  (romacs-agent-rpc-error id code kind text data)))
               (error (romacs-agent-rpc-error id -32603 "internal" (error-message-string err))))))
        (when id (romacs-agent-rpc-send rpc reply)))))))

(defun romacs-agent-service--hello (rpc params)
  "`agents/hello': the version both speak, this service, the caller's identity."
  (let* ((version (gethash "version" params))
         (major (and (vectorp version) (> (length version) 1) (aref version 0)))
         (minor (and major (aref version 1))))
    (unless (and (equal (gethash "protocol" params) "romacs-agents") (integerp major) (integerp minor))
      (romacs-agent-service--fail -32602 "bad-params" "Say protocol romacs-agents and version [MAJOR, MINOR]"))
    (unless (= major (car romacs-agent-service-version))
      (romacs-agent-service--fail -32013 "unsupported" "Another major version"
                                  (romacs-agent-rpc-object "served" (vconcat romacs-agent-service-version)
                                                           "asked" version)))
    (let ((peer (romacs-agent-rpc-data rpc)))
      (setf (romacs-agent-service--peer-hello peer) t)
      (romacs-agent-rpc-object
       "version" (vector major (min minor (cadr romacs-agent-service-version)))
       "server" (romacs-agent-rpc-object "host" (system-name) "build" emacs-version
                                         "generation" romacs-agent-service--generation)
       "identity" (romacs-agent-service--peer-identity peer)
       "capabilities" []))))

;;; Rows

(defun romacs-agent-service--number (value)
  (if (numberp value) value :null))

(defun romacs-agent-service--text (value)
  (if (stringp value) value :null))

(defvar romacs-agent-service--projects (make-hash-table :test #'equal)
  "Each working directory's project name (`romacs-agent-board--project'),
made once: a name is abbreviated through the file system, and a list's
rows share few directories.")

(defun romacs-agent-service--project (cwd)
  (or (gethash cwd romacs-agent-service--projects)
      (puthash cwd (romacs-agent-board--project cwd) romacs-agent-service--projects)))

(defun romacs-agent-service-row (session key children)
  "SESSION as a row (Protocol v1, The session list): KEY its state key on
the board, CHILDREN how many sessions it delegated."
  (let ((options (romacs-agent-options session)))
    (romacs-agent-rpc-object
     "id" (romacs-agent-id session)
     "title" (romacs-agent-service--text (romacs-agent-title session))
     "cwd" (romacs-agent-service--text (romacs-agent-cwd session))
     "project" (romacs-agent-service--project (romacs-agent-cwd session))
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
     "children" children)))

(defun romacs-agent-service-rows ()
  "Every session this machine's board shows, as rows by id: what it shows,
its state keys its own (`romacs-agent-board--key').  It leaves the
board's view of the unloaded sessions made, which a row of one session
needs after (`romacs-agent-service--row')."
  (let* ((sessions (romacs-agent-board--sessions))
         (children (make-hash-table :test #'equal))
         (rows (make-hash-table :test #'equal :size (length sessions))))
    (setq romacs-agent-service--unloaded romacs-agent-board--unloaded)
    (dolist (session sessions)
      (when-let* ((parent (plist-get (romacs-agent-options session) :parent)))
        (puthash parent (1+ (gethash parent children 0)) children)))
    (dolist (session sessions)
      (let ((id (romacs-agent-id session)))
        (unless (gethash id rows)
          (puthash id (romacs-agent-service-row session (romacs-agent-board--key session) (gethash id children 0))
                   rows))))
    rows))

(defvar romacs-agent-service--unloaded nil
  "The board's view of the unloaded sessions as the last whole pass made it
\(`romacs-agent-board--unloaded'), for the rows made between passes.")

(defun romacs-agent-service--row (id)
  "The row of session ID now, its children not counted yet, or nil when it
is gone."
  (let* ((romacs-agent-board--unloaded romacs-agent-service--unloaded)
         (session (or (romacs-agent-loaded id) (romacs-agent-store-row-session id))))
    (and session (romacs-agent-service-row session (romacs-agent-board--key session) 0))))

(defun romacs-agent-service--diff (list updates)
  "Apply UPDATES, each (ID . ROW), ROW nil for a session gone, to LIST:
\(REVISION CHANGED REMOVED), the rows changed and the ids gone, under a
new revision when any; nil when nothing changed."
  (let ((old (romacs-agent-service--list-rows list)) (texts (romacs-agent-service--list-texts list))
        (changed nil) (removed nil))
    (pcase-dolist (`(,id . ,row) updates)
      (if (null row)
          (when (gethash id old) (push id removed))
        (let ((text (json-serialize row)))
          (unless (equal text (gethash id texts))
            (puthash id text texts)
            (push row changed)))))
    (when (or changed removed)
      (let ((revision (cl-incf (romacs-agent-service--list-revision list))))
        (dolist (row changed)
          (let ((id (gethash "id" row)))
            (puthash id row old)
            (puthash id revision (romacs-agent-service--list-since list))
            (remhash id (romacs-agent-service--list-removed list))))
        (dolist (id removed)
          (remhash id old)
          (remhash id texts)
          (remhash id (romacs-agent-service--list-since list))
          (puthash id revision (romacs-agent-service--list-removed list)))
        (list revision changed removed)))))

(defun romacs-agent-service--whole-updates (list)
  "Every row made again, as updates of LIST: each session's, and each id
LIST serves that no session has."
  (let ((rows (romacs-agent-service-rows)) (updates nil))
    (maphash (lambda (id row) (push (cons id row) updates)) rows)
    (maphash (lambda (id _) (unless (gethash id rows) (push (cons id nil) updates)))
             (romacs-agent-service--list-rows list))
    updates))

(defun romacs-agent-service--children-after (rows updates id)
  "How many sessions name ID their parent once UPDATES (each (ID . ROW))
are applied to ROWS."
  (let ((count 0))
    (maphash (lambda (child row)
               (when (and (not (assoc child updates)) (equal (gethash "parent" row) id)) (cl-incf count)))
             rows)
    (dolist (update updates count)
      (when (and (cdr update) (equal (gethash "parent" (cdr update)) id)) (cl-incf count)))))

(defun romacs-agent-service--updates (list ids)
  "The rows of IDS made again, as updates of LIST, with those of their
parents when one of them is new, gone or moved; each counts its children
as the updates leave them."
  (let* ((rows (romacs-agent-service--list-rows list))
         (updates (mapcar (lambda (id) (cons id (romacs-agent-service--row id))) ids))
         (parents nil))
    (pcase-dolist (`(,id . ,row) updates)
      (let ((was (gethash id rows)))
        (unless (and row was (equal (gethash "parent" row) (gethash "parent" was)))
          (dolist (parent (list (and row (gethash "parent" row)) (and was (gethash "parent" was))))
            (when (and (stringp parent) (not (member parent ids))) (cl-pushnew parent parents :test #'equal))))))
    (dolist (parent parents)
      (when-let* ((row (romacs-agent-service--row parent)))
        (push (cons parent row) updates)))
    (pcase-dolist (`(,id . ,row) updates)
      (when row (puthash "children" (romacs-agent-service--children-after rows updates id) row)))
    updates))

(defvar romacs-agent-service--dirty nil "The ids of the sessions whose rows may have changed.")
(defvar romacs-agent-service--store-place nil "Where the store's rows were last looked at (`romacs-agent-rows-since').")
(defvar romacs-agent-service--whole-at 0 "When every row was last made again.")

(defconst romacs-agent-service--whole-every 60
  "Seconds between passes that make every row again, whatever said so.")

(defun romacs-agent-service--refresh (&optional whole)
  "Make the rows that may have changed again (every row when WHOLE, or
when a pass is due), and tell the watchers what changed."
  (setq romacs-agent-service--refresh-timer nil)
  (when romacs-agent-service--list
    (let* ((list romacs-agent-service--list)
           (whole (or whole (> (- (float-time) romacs-agent-service--whole-at) romacs-agent-service--whole-every)))
           (updates (if whole
                        (progn (setq romacs-agent-service--whole-at (float-time)
                                     romacs-agent-service--dirty nil)
                               (romacs-agent-service--whole-updates list))
                      (prog1 (romacs-agent-service--updates list (delete-dups romacs-agent-service--dirty))
                        (setq romacs-agent-service--dirty nil)))))
      (pcase-let ((`(,revision ,changed ,removed) (romacs-agent-service--diff list updates)))
        (when revision (romacs-agent-service--tell revision changed removed))))))

(defun romacs-agent-service--tell (revision changed removed)
  "Tell the watchers the list's REVISION: the rows CHANGED, the ids REMOVED."
  (let ((message (romacs-agent-rpc-notification
                  "agents/sessions.changed"
                  (romacs-agent-rpc-object "revision" revision "rows" (vconcat changed)
                                           "removed" (vconcat removed)))))
    ;; Too big for one message: its watchers ask for what changed.
    (when (> (string-bytes (json-serialize message)) romacs-agent-rpc-message-limit)
      (setq message (romacs-agent-rpc-notification
                     "agents/sessions.changed"
                     (romacs-agent-rpc-object "revision" revision "stale" t))))
    (dolist (rpc romacs-agent-service--peers)
      (when (romacs-agent-service--peer-watching (romacs-agent-rpc-data rpc))
        (romacs-agent-rpc-send rpc message)))))

(defun romacs-agent-service--refresh-soon ()
  (unless romacs-agent-service--refresh-timer
    (setq romacs-agent-service--refresh-timer (run-at-time 0.1 nil #'romacs-agent-service--refresh))))

(defun romacs-agent-service--event (session _event)
  "SESSION raised an event: its row may have changed."
  (push (romacs-agent-id session) romacs-agent-service--dirty)
  (romacs-agent-service--refresh-soon))

(defun romacs-agent-service--poll ()
  "The rows other processes wrote since the last look may have changed."
  (when romacs-agent-rows-since
    (pcase-let ((`(,ids . ,place) (funcall romacs-agent-rows-since romacs-agent-service--store-place)))
      (setq romacs-agent-service--store-place place)
      (setq romacs-agent-service--dirty (append ids romacs-agent-service--dirty))))
  (romacs-agent-service--refresh-soon))

;;; Watching, paging

(defun romacs-agent-service--watch (rpc params)
  "`agents/sessions.watch': every row, or those changed since the revision
SINCE of this generation, and the ids removed since."
  (let* ((list romacs-agent-service--list)
         (since (gethash "since" params))
         (since (and (integerp since) (<= since (romacs-agent-service--list-revision list)) since))
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
whole rows up to the budget, at least one, and a token continuing it."
  (let ((budget romacs-agent-service--unit-budget) (taken nil)
        (rows (plist-get state :rows)))
    (while (and rows (or (null taken) (<= (string-bytes (json-serialize (car rows))) budget)))
      (setq budget (- budget (string-bytes (json-serialize (car rows)))))
      (push (pop rows) taken))
    (romacs-agent-rpc-object
     "revision" (plist-get state :revision) "reset" (if (plist-get state :reset) t :false)
     "rows" (vconcat (nreverse taken))
     "removed" (if rows [] (vconcat (plist-get state :removed)))
     ;; The rest continues this reply: no reset again.
     "more" (if rows
                (romacs-agent-service--token rpc (plist-put (plist-put (copy-sequence state) :rows rows) :reset nil))
              :null))))

(defun romacs-agent-service--token (rpc state)
  "A continuation token of RPC's for STATE; its oldest token dies when it
holds too many."
  (let* ((peer (romacs-agent-rpc-data rpc))
         (token (substring (romacs-agent-service--uuid) 0 18))
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

(provide 'romacs-agent-service)
;;; romacs-agent-service.el ends here
