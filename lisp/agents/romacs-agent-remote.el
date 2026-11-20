;;; romacs-agent-remote.el --- Another machine's agent sessions, through its service  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Romacs on another machine (doc/agent-protocol.org, Clients): the
;; agent sessions of the machine `romacs-agent-remote-host' names, drawn
;; here as this editor draws its own.  `romacs-relay connect' carries the
;; protocol from that machine's service to a socket here; a client
;; (romacs-agent-client.el) mirrors what the service serves; and this is
;; a chat's source (`romacs-agent-log--source') made of that client.
;;
;; A session here is a mirror: a session record made from the row the
;; service sends, never one of this editor's (`romacs-agent--sessions'),
;; never folded, its events never raised.  A chat of one is filled from
;; the service's history and follows the units it sends live; the
;; drawing is the chat's own.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent)
(require 'romacs-agent-client)
(require 'romacs-agent-log)
(require 'romacs-agent-board)
(require 'romacs-agent-wire)

(defgroup romacs-agent-remote nil
  "Another machine's agent sessions."
  :group 'romacs-agent)

(defcustom romacs-agent-remote-host nil
  "The machine whose agent sessions this editor shows: its relay's URL,
as `romacs-relay connect' takes it (wss://HOST/agents), or nil."
  :type '(choice (const :tag "None" nil) string) :group 'romacs-agent-remote)

(defcustom romacs-agent-remote-relay "romacs-relay"
  "The `romacs-relay' program."
  :type 'string :group 'romacs-agent-remote)

(cl-defstruct (romacs-agent-remote (:constructor romacs-agent-remote--make) (:copier nil))
  "Another machine's sessions as a chat's source: the CLIENT of its
service, the RELAY process that carries it here, the mirror of each of
its sessions, by id (SESSIONS), and their CHATS by id."
  client relay
  (sessions (make-hash-table :test #'equal))
  (chats (make-hash-table :test #'equal)))

(defvar romacs-agent-remote--source nil
  "The other machine this editor shows the sessions of, or nil: one at a time.")

;;; Mirrors

(defun romacs-agent-remote--value (value)
  (unless (memq value '(:null :false)) value))

(defun romacs-agent-remote--mirror (source row)
  "SOURCE's session of ROW, a row the service sent, made or brought up to
date from it: the same record for its id each time."
  (let* ((id (gethash "id" row))
         (session (or (gethash id (romacs-agent-remote-sessions source))
                      (puthash id (romacs-agent--make :id id) (romacs-agent-remote-sessions source))))
         (get (lambda (key) (romacs-agent-remote--value (gethash key row)))))
    (setf (romacs-agent-title session) (funcall get "title")
          (romacs-agent-cwd session) (funcall get "cwd")
          (romacs-agent-harness session) (and (funcall get "harness") (intern (funcall get "harness")))
          (romacs-agent-provider session) (funcall get "provider")
          (romacs-agent-model session) (funcall get "model")
          (romacs-agent-options session) (append (and (funcall get "parent") (list :parent (funcall get "parent")))
                                                 (and (funcall get "effort") (list :effort (funcall get "effort"))))
          (romacs-agent-state session) (intern (or (funcall get "state") "detached"))
          (romacs-agent-last-time session) (funcall get "last_time")
          (romacs-agent-cost session) (funcall get "cost")
          (romacs-agent-context-used session) (funcall get "context_used")
          (romacs-agent-context-window session) (funcall get "context_window")
          (romacs-agent-activity session) (funcall get "activity")
          (romacs-agent-turn-start session) (funcall get "turn_start")
          (romacs-agent-input-seq session) (funcall get "input_seq")
          (romacs-agent-done-seq session) (or (funcall get "done_seq") 0)
          (romacs-agent-seen-seq session) (funcall get "seen_seq")
          (romacs-agent-settled-seq session) (funcall get "settled_seq"))
    session))

;;; The chat

;; A chat of another machine's session draws what its client tells of
;; following it (`romacs-agent-client-follow'): all of it afresh, then
;; the units that follow.

(defun romacs-agent-remote--chat (source id)
  "The chat of SOURCE's session ID, when it has one."
  (when-let* ((buffer (gethash id (romacs-agent-remote-chats source))) ((buffer-live-p buffer)))
    buffer))

(defun romacs-agent-remote--draw-unit (session unit &optional live)
  "Draw UNIT, a record unit the service sent, in the current chat: LIVE,
as its live events are drawn."
  (if (eq (gethash "oversize" unit) t)
      (romacs-agent-log--writing (romacs-agent-log--note nil "a record too large to send"))
    (dolist (event (romacs-agent-wire-decode-unit (gethash "events" unit)))
      (if live (romacs-agent-log--draw-live session event) (romacs-agent-log-draw session event))))
  (setq romacs-agent-log--seq (max romacs-agent-log--seq (gethash "seq" unit))))

(defun romacs-agent-remote--drawn (session units from)
  "Draw SESSION afresh: UNITS, beginning at seq FROM (nil, its start)."
  (romacs-agent-log--clear)
  (setq romacs-agent-log--from from)
  (when from
    (romacs-agent-log--writing (save-excursion (goto-char (point-min)) (insert (romacs-agent-log--earlier-line from)))))
  (dolist (unit units) (romacs-agent-remote--draw-unit session unit))
  (romacs-agent-log--close-stream)
  (romacs-agent-log--sync-asks session)
  (romacs-agent-log--separator t)
  (romacs-agent-log--header-show))

(defun romacs-agent-remote--followed (client id notice)
  "What came of following session ID on CLIENT, in its chat and board."
  (when-let* ((source romacs-agent-remote--source)
              ((eq client (romacs-agent-remote-client source))))
    (pcase notice
      (`(row ,row) (romacs-agent-remote--mirror source row))
      (`(pending ,requests) (romacs-agent-remote--waits source id requests)))
    (when-let* ((buffer (romacs-agent-remote--chat source id)))
      (with-current-buffer buffer
        (let ((session romacs-agent-log--session))
          (pcase notice
            (`(drawn ,units ,from) (romacs-agent-remote--drawn session units from))
            (`(more ,units) (dolist (unit units) (romacs-agent-remote--draw-unit session unit t)))
            (`(row ,_) (romacs-agent-log--separator) (romacs-agent-log--header-show))
            (`(pending ,_)
             (when romacs-agent-log--filled (romacs-agent-log--sync-asks session))
             (romacs-agent-log--separator))
            (`(failed ,error)
             (romacs-agent-log--writing
               (romacs-agent-log--note nil (format "not followed: %s" (gethash "message" error)))))
            ('(ended)
             (romacs-agent-log--writing
               (romacs-agent-log--note nil (substitute-command-keys
                                            "the connection ended: \\[romacs-agents-remote] connects again"))))))))))

(add-hook 'romacs-agent-client-follow-functions #'romacs-agent-remote--followed)

(cl-defmethod romacs-agent-log-source-fill ((source romacs-agent-remote) session)
  "Follow SESSION on SOURCE's service, drawn as it is told; no longer once
the chat is killed."
  (let ((id (romacs-agent-id session)))
    (add-hook 'kill-buffer-hook
              (lambda ()
                (remhash id (romacs-agent-remote-chats source))
                (when-let* ((client (romacs-agent-remote-client source))) (romacs-agent-client-unfollow client id)))
              nil t)
    (romacs-agent-client-follow (romacs-agent-remote-client source) id romacs-agent-log-turns romacs-agent-log-records)))

(cl-defmethod romacs-agent-log-source-earlier ((source romacs-agent-remote) session)
  "Draw SESSION's turns before those the chat shows, read from its service."
  (let ((end romacs-agent-log--from) (buffer (current-buffer)))
    (unless end (user-error "This chat shows the session from its start"))
    (romacs-agent-client-earlier
     (romacs-agent-remote-client source) (romacs-agent-id session) (1- end)
     romacs-agent-log-turns romacs-agent-log-records
     (lambda (result)
       (when (buffer-live-p buffer)
         (with-current-buffer buffer
           (pcase result
             (`(error ,error) (message "Earlier turns not read: %s" (gethash "message" error)))
             ('(reset) (message "Its history moved: the chat is drawn afresh"))
             (`(,units ,from)
              (romacs-agent-log--draw-above
               session from (lambda () (dolist (unit units) (romacs-agent-remote--draw-unit session unit))))))))))))

(defun romacs-agent-remote--waits (source id requests)
  "SOURCE's session ID waits on REQUESTS, a table of handle id to request:
its mirror's approvals, each the approval event that asks it."
  (when-let* ((session (gethash id (romacs-agent-remote-sessions source))))
    (let ((approvals (make-hash-table :test #'equal)))
      (maphash (lambda (handle request)
                 (puthash handle (append (list :type 'approval) (romacs-agent-wire--asked request)) approvals))
               requests)
      (setf (romacs-agent-approvals session) approvals))))

(cl-defmethod romacs-agent-log-source-request-event ((_source romacs-agent-remote) _id request)
  "REQUEST, of a mirror's approvals, is its approval event."
  request)

(cl-defmethod romacs-agent-log-source-answers-p ((_source romacs-agent-remote) _session event)
  "Whether the policy of the editor serving EVENT answers it, as it said."
  (eq (plist-get event :policy) t))

(cl-defmethod romacs-agent-log-source-chats ((source romacs-agent-remote))
  (romacs-agent-remote-chats source))

(cl-defmethod romacs-agent-log-source-approvals ((_source romacs-agent-remote) session)
  (romacs-agent-approvals session))

(cl-defmethod romacs-agent-log-source-seen ((_source romacs-agent-remote) _session)
  nil)

(defun romacs-agent-remote--no-commands (&rest _)
  (user-error "Another machine's sessions are shown, not yet acted on: its service takes no commands"))

(cl-defmethod romacs-agent-log-source-send ((_source romacs-agent-remote) _session _text &optional _when)
  (romacs-agent-remote--no-commands))
(cl-defmethod romacs-agent-log-source-interrupt ((_source romacs-agent-remote) _session)
  (romacs-agent-remote--no-commands))
(cl-defmethod romacs-agent-log-source-respond ((_source romacs-agent-remote) _session _id _allow _reason)
  (romacs-agent-remote--no-commands))
(cl-defmethod romacs-agent-log-source-ask ((_source romacs-agent-remote) _session _id _questions)
  (romacs-agent-remote--no-commands))

;;; The board

(defun romacs-agent-remote--rows (source)
  "The rows SOURCE's service lists, by id: none until it is connected."
  (if-let* ((client (romacs-agent-remote-client source))) (romacs-agent-client-rows client)
    (make-hash-table :test #'equal)))

(cl-defmethod romacs-agent-board-source-sessions ((source romacs-agent-remote))
  "A mirror of each row SOURCE's service lists; none is loaded here, so
the board knows them all as shown from their rows."
  (let ((by-id (make-hash-table :test #'equal)) (children (make-hash-table :test #'equal)) (sessions nil))
    (maphash (lambda (_ row)
               (let ((session (romacs-agent-remote--mirror source row)))
                 (push session sessions)
                 (puthash (romacs-agent-id session) session by-id)
                 (when-let* ((parent (plist-get (romacs-agent-options session) :parent)))
                   (push session (gethash parent children)))))
             (romacs-agent-remote--rows source))
    (setq romacs-agent-board--unloaded (cons by-id children))
    sessions))

(cl-defmethod romacs-agent-board-source-key ((source romacs-agent-remote) session)
  "The key SOURCE's service made for SESSION: drawn, never derived here."
  (let ((key (gethash "key" (gethash (romacs-agent-id session) (romacs-agent-remote--rows source)))))
    (if (stringp key) (intern key) 'idle)))

(cl-defmethod romacs-agent-board-source-live ((source romacs-agent-remote) session)
  (and (gethash (romacs-agent-id session) (romacs-agent-remote--rows source)) session))

(defun romacs-agent-remote--rows-changed (client _changed _removed)
  "CLIENT's list changed: the board shows it again."
  (when (and romacs-agent-remote--source (eq client (romacs-agent-remote-client romacs-agent-remote--source)))
    (romacs-agent-board--refresh)))

(add-hook 'romacs-agent-client-rows-functions #'romacs-agent-remote--rows-changed)

;;; Connecting

(defun romacs-agent-remote--socket ()
  (expand-file-name "remote.sock" (romacs-agent--runtime-root)))

(defun romacs-agent-remote-connect (&optional socket)
  "Show another machine's agent sessions: connect to the service whose
SOCKET is given, else start `romacs-relay connect' to
`romacs-agent-remote-host' and connect through it once it listens, the
editor not waiting.  Connected again, the chats it shows are followed
afresh.  The source."
  (let ((source (or romacs-agent-remote--source (romacs-agent-remote--make))))
    (setq romacs-agent-remote--source source romacs-agent-board-source source)
    (when (romacs-agent-remote-client source) (romacs-agent-client-close (romacs-agent-remote-client source)))
    (setf (romacs-agent-remote-client source) nil)
    (if socket
        (romacs-agent-remote--connected source (romacs-agent-client-connect socket))
      (unless romacs-agent-remote-host (user-error "No machine named: set `romacs-agent-remote-host'"))
      (unless (process-live-p (romacs-agent-remote-relay source))
        ;; Its socket's directory the relay's to make: the user's alone.
        (setf (romacs-agent-remote-relay source)
              (make-process :name "romacs-relay" :buffer (generate-new-buffer " *romacs-relay*") :noquery t
                            :command (list romacs-agent-remote-relay "connect" romacs-agent-remote-host
                                           "--socket" (romacs-agent-remote--socket)))))
      (romacs-agent-remote--dial source (+ (float-time) 10)))
    source))

(defun romacs-agent-remote--dial (source deadline)
  "Connect SOURCE through its relay once it listens, trying again a moment
later until DEADLINE: a socket there before it, a stale one, refuses
until the relay replaces it."
  (let ((relay (romacs-agent-remote-relay source)))
    (when (eq source romacs-agent-remote--source)
      (if-let* ((client (ignore-error file-error (romacs-agent-client-connect (romacs-agent-remote--socket)))))
          (romacs-agent-remote--connected source client)
        (if (and (process-live-p relay) (< (float-time) deadline))
            (run-at-time 0.05 nil #'romacs-agent-remote--dial source deadline)
          (when (process-live-p relay) (delete-process relay))
          (message "No relay to %s: %s" romacs-agent-remote-host
                   (with-current-buffer (process-buffer relay) (string-trim (buffer-string)))))))))

(defun romacs-agent-remote--connected (source client)
  "SOURCE connected through CLIENT: each chat it shows followed through it."
  (setf (romacs-agent-remote-client source) client)
  (maphash (lambda (id buffer)
             (when (buffer-live-p buffer)
               (romacs-agent-client-follow client id romacs-agent-log-turns romacs-agent-log-records)))
           (romacs-agent-remote-chats source)))

(defun romacs-agent-remote-disconnect ()
  "Stop showing another machine's sessions."
  (when-let* ((source romacs-agent-remote--source))
    (setq romacs-agent-remote--source nil romacs-agent-board-source romacs-agent-log-local)
    (when (romacs-agent-remote-client source) (romacs-agent-client-close (romacs-agent-remote-client source)))
    (when (process-live-p (romacs-agent-remote-relay source)) (delete-process (romacs-agent-remote-relay source)))))

(defun romacs-agent-remote--client ()
  "The client of the other machine shown, connected, or a user error."
  (let ((client (and romacs-agent-remote--source (romacs-agent-remote-client romacs-agent-remote--source))))
    (unless (and client (romacs-agent-client-live-p client))
      (user-error (substitute-command-keys "Not connected: \\[romacs-agents-remote] connects")))
    client))

(defun romacs-agents-remote ()
  "Show the agent board of the machine `romacs-agent-remote-host' names:
its sessions, drawn from its agents service; connected again when the
connection ended."
  (interactive)
  (let ((client (and romacs-agent-remote--source (romacs-agent-remote-client romacs-agent-remote--source))))
    (unless (and client (romacs-agent-client-live-p client)) (romacs-agent-remote-connect)))
  (romacs-agents))

(defun romacs-agent-remote-chat (id)
  "The chat of the other machine's session ID, drawn from its service."
  (let* ((client (romacs-agent-remote--client)) (source romacs-agent-remote--source)
         (row (or (gethash id (romacs-agent-client-rows client)) (user-error "No session %s there" id))))
    (romacs-agent-log-chat (romacs-agent-remote--mirror source row) nil source)))

(provide 'romacs-agent-remote)
;;; romacs-agent-remote.el ends here
