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
          (romacs-agent-context-window session) (funcall get "context_window"))
    session))

;;; The chat

(defvar-local romacs-agent-remote--held nil
  "While the chat is filled or a gap pulled, the live units that came
meanwhile, newest first, behind `held'; nil once it follows them.")

(defun romacs-agent-remote--chat (source id)
  "The chat of SOURCE's session ID, when it has one."
  (when-let* ((buffer (gethash id (romacs-agent-remote-chats source))) ((buffer-live-p buffer)))
    buffer))

(defun romacs-agent-remote--draw-unit (session unit)
  "Draw UNIT, a record unit the service sent, in the current chat."
  (if (eq (gethash "oversize" unit) t)
      (romacs-agent-log--note nil "a record too large to send")
    (dolist (event (romacs-agent-wire-decode-unit (gethash "events" unit)))
      (romacs-agent-log-draw session event)))
  (setq romacs-agent-log--seq (max romacs-agent-log--seq (gethash "seq" unit))))

(defun romacs-agent-remote--follow (source session)
  "Draw the live units held while the chat was filled, past what it drew,
then follow them as they come."
  (let ((held (reverse (cdr romacs-agent-remote--held))))
    (setq romacs-agent-remote--held nil)
    (dolist (params held) (romacs-agent-remote--live source session params))))

(defun romacs-agent-remote--history (source session params then)
  "Pull SESSION's history PARAMS asks for, then call THEN in its chat
with (UNITS CURSOR RESET FROM), or (error ERROR)."
  (let ((buffer (current-buffer)))
    (romacs-agent-client-history
     (romacs-agent-remote-client source) (romacs-agent-id session) params
     (lambda (result) (when (buffer-live-p buffer) (with-current-buffer buffer (funcall then result)))))))

(defun romacs-agent-remote--filled (source session result)
  "Draw RESULT, the history the chat opens with, then what came live."
  (romacs-agent-log--writing
    (pcase result
      (`(error ,error) (romacs-agent-log--note nil (format "history not read: %s" (gethash "message" error))))
      (`(,units ,_cursor ,reset ,from)
       (when reset (romacs-agent-log--note nil "history not read: it moved while read"))
       (setq romacs-agent-log--from from)
       (when from (save-excursion (goto-char (point-min)) (insert (romacs-agent-log--earlier-line from))))
       (dolist (unit units) (romacs-agent-remote--draw-unit session unit))
       (romacs-agent-log--close-stream))))
  (romacs-agent-log--sync-asks session)
  (romacs-agent-remote--follow source session)
  (romacs-agent-log--separator t)
  (romacs-agent-log--header-show))

(defun romacs-agent-remote--live (source session params)
  "Draw PARAMS, an `agents/event' of SESSION, in its chat: held while
history is pulled; a gap pulled as history, the units after it held."
  (cond
   (romacs-agent-remote--held (push params (cdr romacs-agent-remote--held)))
   ((hash-table-p (gethash "gap" params))
    (let ((gap (gethash "gap" params)))
      (setq romacs-agent-remote--held (list 'held))
      (romacs-agent-remote--history
       source session (romacs-agent-rpc-object "after" (max romacs-agent-log--seq (gethash "after" gap))
                                               "upto" (gethash "upto" gap))
       (lambda (result)
         (pcase result
           (`(,units ,_ nil ,_) (dolist (unit units) (romacs-agent-remote--live-unit session unit)))
           (_ (romacs-agent-log--note nil "records missed: open the chat again")))
         (romacs-agent-remote--follow source session)))))
   (t (romacs-agent-remote--live-unit session params))))

(defun romacs-agent-remote--live-unit (session unit)
  "Draw UNIT's events live, as the chat draws its own session's."
  (let ((seq (gethash "seq" unit)))
    (when (> seq romacs-agent-log--seq)
      (if (eq (gethash "oversize" unit) t)
          (romacs-agent-remote--draw-unit session unit)
        (dolist (event (romacs-agent-wire-decode-unit (gethash "events" unit)))
          (romacs-agent-log--draw-live session event)))
      (setq romacs-agent-log--seq (max romacs-agent-log--seq seq)))))

(defun romacs-agent-remote--notice (client id notice)
  "What came of session ID on CLIENT, drawn in its chat."
  (when-let* ((source romacs-agent-remote--source)
              ((eq client (romacs-agent-remote-client source)))
              (buffer (romacs-agent-remote--chat source id)))
    (with-current-buffer buffer
      (let ((session romacs-agent-log--session))
        (pcase notice
          (`(opened ,reply)
           (when (hash-table-p (gethash "row" reply)) (romacs-agent-remote--mirror source (gethash "row" reply)))
           (romacs-agent-remote--history
            source session (romacs-agent-rpc-object "upto" (gethash "seq" (gethash "cursor" reply))
                                                    "turns" romacs-agent-log-turns
                                                    "records" romacs-agent-log-records)
            (lambda (result) (romacs-agent-remote--filled source session result))))
          (`(event ,params) (romacs-agent-remote--live source session params))
          (`(changed ,params)
           (romacs-agent-remote--mirror source (gethash "row" params))
           (romacs-agent-log--separator)
           (romacs-agent-log--header-show))
          (`(failed ,error)
           (romacs-agent-log--writing
             (romacs-agent-log--note nil (format "not opened: %s" (gethash "message" error))))))))))

(add-hook 'romacs-agent-client-session-functions #'romacs-agent-remote--notice)

(cl-defmethod romacs-agent-log-source-fill ((source romacs-agent-remote) session)
  "Open SESSION on SOURCE's service: its history drawn once it answers,
the live units that come meanwhile held for after it."
  (setq romacs-agent-remote--held (list 'held))
  (romacs-agent-client-open (romacs-agent-remote-client source) (romacs-agent-id session)))

(cl-defmethod romacs-agent-log-source-chats ((source romacs-agent-remote))
  (romacs-agent-remote-chats source))

(cl-defmethod romacs-agent-log-source-approvals ((_source romacs-agent-remote) session)
  (romacs-agent-approvals session))

(cl-defmethod romacs-agent-log-source-seen ((_source romacs-agent-remote) _session)
  nil)

(defun romacs-agent-remote--no-commands (&rest _)
  (user-error "Another machine's sessions are shown, not yet acted on: its service takes no commands"))

(cl-defmethod romacs-agent-log-source-earlier ((_source romacs-agent-remote) _session)
  (romacs-agent-remote--no-commands))
(cl-defmethod romacs-agent-log-source-send ((_source romacs-agent-remote) _session _text &optional _when)
  (romacs-agent-remote--no-commands))
(cl-defmethod romacs-agent-log-source-interrupt ((_source romacs-agent-remote) _session)
  (romacs-agent-remote--no-commands))
(cl-defmethod romacs-agent-log-source-respond ((_source romacs-agent-remote) _session _id _allow _reason)
  (romacs-agent-remote--no-commands))
(cl-defmethod romacs-agent-log-source-ask ((_source romacs-agent-remote) _session _id _questions)
  (romacs-agent-remote--no-commands))

;;; The board

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
             (romacs-agent-client-rows (romacs-agent-remote-client source)))
    (setq romacs-agent-board--unloaded (cons by-id children))
    sessions))

(cl-defmethod romacs-agent-board-source-key ((source romacs-agent-remote) session)
  "The key SOURCE's service made for SESSION: drawn, never derived here."
  (let ((key (gethash "key" (gethash (romacs-agent-id session) (romacs-agent-client-rows (romacs-agent-remote-client source))))))
    (if (stringp key) (intern key) 'idle)))

(cl-defmethod romacs-agent-board-source-live ((source romacs-agent-remote) session)
  (and (gethash (romacs-agent-id session) (romacs-agent-client-rows (romacs-agent-remote-client source))) session))

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
`romacs-agent-remote-host' and connect through it."
  (when romacs-agent-remote--source (romacs-agent-remote-disconnect))
  (let* ((relay (unless socket
                  (unless romacs-agent-remote-host (user-error "No machine named: set `romacs-agent-remote-host'"))
                  ;; Its directory the relay's to make: the user's alone.
                  (setq socket (romacs-agent-remote--socket))
                  (make-process :name "romacs-relay" :buffer " *romacs-relay*" :noquery t
                                :command (list romacs-agent-remote-relay "connect" romacs-agent-remote-host
                                               "--socket" socket))))
         (deadline (+ (float-time) 10))
         ;; The relay's socket answers once it listens: one there before
         ;; it, a stale one, refuses until the relay replaces it.
         (client (if (not relay) (romacs-agent-client-connect socket)
                   (let ((client nil))
                     (while (and (not client) (process-live-p relay) (< (float-time) deadline))
                       (unless (setq client (ignore-error file-error (romacs-agent-client-connect socket)))
                         (accept-process-output relay 0.05)))
                     (or client
                         (progn (when (process-live-p relay) (delete-process relay))
                                (user-error "No relay to %s: %s" romacs-agent-remote-host
                                            (with-current-buffer (process-buffer relay)
                                              (string-trim (buffer-string))))))))))
    (setq romacs-agent-remote--source (romacs-agent-remote--make :client client :relay relay)
          romacs-agent-board-source romacs-agent-remote--source)))

(defun romacs-agent-remote-disconnect ()
  "Stop showing another machine's sessions."
  (when-let* ((source romacs-agent-remote--source))
    (setq romacs-agent-remote--source nil romacs-agent-board-source romacs-agent-log-local)
    (romacs-agent-client-close (romacs-agent-remote-client source))
    (when (process-live-p (romacs-agent-remote-relay source)) (delete-process (romacs-agent-remote-relay source)))))

(defun romacs-agents-remote ()
  "Show the agent board of the machine `romacs-agent-remote-host' names:
its sessions, drawn from its agents service."
  (interactive)
  (unless romacs-agent-remote--source (romacs-agent-remote-connect))
  (romacs-agents))

(defun romacs-agent-remote-chat (id)
  "The chat of the other machine's session ID, drawn from its service."
  (let* ((source (or romacs-agent-remote--source (user-error "No other machine connected")))
         (row (or (gethash id (romacs-agent-client-rows (romacs-agent-remote-client source)))
                  (user-error "No session %s there" id))))
    (romacs-agent-log-chat (romacs-agent-remote--mirror source row) nil source)))

(provide 'romacs-agent-remote)
;;; romacs-agent-remote.el ends here
