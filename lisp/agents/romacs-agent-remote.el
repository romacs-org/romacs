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
its sessions, by id (SESSIONS), their CHATS by id, while it connects the
DIAL its one attempt is named by, and the PROJECTS and PROVIDERS its
service last listed (`agents/projects.list', `agents/providers.list')."
  client relay dial projects providers
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
          (romacs-agent-settled-seq session) (funcall get "settled_seq")
          ;; A service that says no `failed' says so by its key.
          (romacs-agent-failed session) (or (eq (gethash "failed" row) t) (equal (gethash "key" row) "failed")))
    session))

;;; The chat

;; A chat of another machine's session draws what its client tells of
;; following it (`romacs-agent-client-follow'): all of it afresh, then
;; the units that follow.

(defun romacs-agent-remote--chat (source id)
  "The chat of SOURCE's session ID, when it has one."
  (when-let* ((buffer (gethash id (romacs-agent-remote-chats source))) ((buffer-live-p buffer)))
    buffer))

(defun romacs-agent-remote--draw-unit (session unit &optional source)
  "Draw UNIT from the service.  SOURCE is `live', `snapshot', or nil for history."
  (let ((seq (gethash "seq" unit)))
    (when (or (not (eq source 'live)) (> seq romacs-agent-log--seq))
      (if (eq (gethash "oversize" unit) t)
          (romacs-agent-log--writing (romacs-agent-log--note nil "a record too large to send"))
        (dolist (event (romacs-agent-wire-decode-unit (gethash "events" unit)))
          (when source (romacs-agent-ui-track-workers session event))
          (if (eq source 'live) (romacs-agent-log--draw-live session event) (romacs-agent-log-draw session event))))
      (setq romacs-agent-log--seq (max romacs-agent-log--seq seq)))))

(defun romacs-agent-remote--drawn (session units from)
  "Draw SESSION afresh: UNITS, beginning at seq FROM (nil, its start)."
  (romacs-agent-log--clear)
  (romacs-agent-ui-reset-workers session)
  (setq romacs-agent-log--from from romacs-agent-remote--earlier nil)
  (when from
    (romacs-agent-log--writing (save-excursion (goto-char (point-min)) (insert (romacs-agent-log--earlier-line from)))))
  ;; What streams at its end goes on streaming: the live units that
  ;; follow may carry on its message.
  (let ((romacs-agent-log--closes-deferred t))
    (dolist (unit units) (romacs-agent-remote--draw-unit session unit 'snapshot)))
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
            (`(more ,units) (dolist (unit units) (romacs-agent-remote--draw-unit session unit 'live)))
            (`(row ,_) (romacs-agent-log--separator) (romacs-agent-log--header-show))
            (`(pending ,_)
             (when romacs-agent-log--filled (romacs-agent-log--sync-asks session))
             (romacs-agent-log--separator))
            (`(failed ,error)
             (setq romacs-agent-remote--earlier nil)
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

(defvar-local romacs-agent-remote--earlier nil
  "While the chat's earlier turns are read, the read's own token: (END),
END where the chat began when they were asked for.")

(cl-defmethod romacs-agent-log-source-earlier ((source romacs-agent-remote) session)
  "Draw SESSION's turns before those the chat shows, read from its service:
once, through the chat's own source, and only while the chat still
begins where it did; its history gone, the chat is drawn afresh."
  (let* ((end romacs-agent-log--from) (buffer (current-buffer)) (id (romacs-agent-id session))
         (client (romacs-agent-remote-client source)) (token (list end)) (sent nil))
    (unless end (user-error "This chat shows the session from its start"))
    (when romacs-agent-remote--earlier (user-error "Its earlier turns are being read"))
    (unless (and client (romacs-agent-client-live-p client))
      (user-error (substitute-command-keys "Not connected: \\[romacs-agents-remote] connects")))
    (setq romacs-agent-remote--earlier token)
    ;; Its token is let go of however this exits, a quit too, until the
    ;; read is sent: then its reply's.
    (unwind-protect
        (setq sent (romacs-agent-client-earlier
                    client id (1- end) romacs-agent-log-turns romacs-agent-log-records
                    (lambda (result) (romacs-agent-remote--earlier-read buffer source client session token result))))
      (unless sent (when (eq romacs-agent-remote--earlier token) (setq romacs-agent-remote--earlier nil))))
    (unless sent (user-error "This chat is not followed: open it again"))))

(defun romacs-agent-remote--earlier-read (buffer source client session token result)
  "RESULT, of the earlier turns BUFFER's read TOKEN asked for, drawn while
TOKEN is still its read, its chat beginning where it did and SOURCE's
client still CLIENT."
  (when (and (buffer-live-p buffer) (eq (buffer-local-value 'romacs-agent-remote--earlier buffer) token))
    (with-current-buffer buffer
      (let ((current (and (eql (car token) romacs-agent-log--from) (eq client (romacs-agent-remote-client source)))))
        (setq romacs-agent-remote--earlier nil)
        (pcase result
          (`(error ,error) (message "Earlier turns not read: %s" (gethash "message" error)))
          ('(reset) (when current (romacs-agent-client-open client (romacs-agent-id session))))
          (`(,units ,from)
           (when current
             (romacs-agent-log--draw-above
              session from (lambda () (dolist (unit units) (romacs-agent-remote--draw-unit session unit)))))))))))

(defun romacs-agent-remote--waits (source id requests)
  "SOURCE's session ID waits on REQUESTS, a table of handle id to request:
its mirror's approvals, each the approval event that asks it."
  (when-let* ((session (gethash id (romacs-agent-remote-sessions source))))
    (let ((approvals (make-hash-table :test #'equal)))
      (maphash (lambda (handle request)
                 (puthash handle (append (list :type 'approval) (romacs-agent-wire--asked request)) approvals))
               requests)
      (setf (romacs-agent-approvals session) approvals))))

(cl-defmethod romacs-agent-log-source-request-event ((_source romacs-agent-remote) _session _id request)
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

(defun romacs-agent-remote--command (source method params &optional done)
  "Run METHOD with PARAMS on SOURCE's service, then call DONE, when given,
with its result and nil, or nil and its error; with no DONE, an error is
said.  `user-error' when SOURCE is not connected."
  (let ((client (romacs-agent-remote-client source)))
    (unless (and client (romacs-agent-client-live-p client))
      (user-error (substitute-command-keys "Not connected: \\[romacs-agents-remote] connects")))
    (romacs-agent-client-command client method params (or done (romacs-agent-remote--said method)))))

(defun romacs-agent-remote--said (method)
  "A reply's callback for METHOD that says its error, should it fail."
  (lambda (_result error)
    (when error
      (let* ((data (gethash "data" error))
             (unconfirmed (and (hash-table-p data) (member (gethash "kind" data) '("not-kept" "disconnected")))))
        (message "%s%s — %s" method (if unconfirmed " delivery unconfirmed" "") (gethash "message" error))))))

(defconst romacs-agent-remote--unsure-kinds '("disconnected" "not-kept")
  "The error kinds of a command that may have taken effect: the connection
ended before its reply, or its outcome was not kept.")

(defun romacs-agent-remote--unconfirmed (intent minor)
  "The answer to INTENT, sent to a service of protocol 1.MINOR, when it is
not known whether it went: neither accepted nor refused, its draft kept
under its id.  From 1.1 a retry goes once while the service runs."
  (list :draft-id (plist-get intent :draft-id) :status 'unconfirmed
        :message (if (>= minor 1) "Unconfirmed: send again, it goes once" "The send is unconfirmed")))

(defun romacs-agent-remote--submit-answer (intent result error &optional minor)
  "The answer to INTENT, sent as `agents/send' to a service of protocol
1.MINOR (1.1 by default) and answered with RESULT, or ERROR: accepted
when RESULT says it was written or queued, with :queued N when it says
its place; unconfirmed when RESULT says another state (unknown) or
ERROR says it may have taken effect (`romacs-agent-remote--unsure-kinds');
else refused, nothing done, for the reason its kind says
\(`romacs-agent-submit-reason')."
  (let* ((data (and error (gethash "data" error)))
         (kind (and (hash-table-p data) (gethash "kind" data))))
    (cond ((and (not error) (member (and (hash-table-p result) (gethash "state" result)) '("written" "queued")))
           (let ((place (gethash "queued" result)))
             (append (list :draft-id (plist-get intent :draft-id) :status 'accepted)
                     (and (integerp place) (list :queued place)))))
          ;; A result that says no state it took, as a command kept
          ;; before its outcome was (unknown), may have gone.
          ((not error) (romacs-agent-remote--unconfirmed intent (or minor 1)))
          ((member kind romacs-agent-remote--unsure-kinds) (romacs-agent-remote--unconfirmed intent (or minor 1)))
          (t (romacs-agent-submit-refused intent (romacs-agent-submit-reason kind) (gethash "message" error))))))

(defun romacs-agent-remote--send-params (session intent minor)
  "`agents/send''s params for INTENT to SESSION, to a service of protocol
1.MINOR: before 1.1, which has no interrupt mode, an interrupt's text
goes as next."
  (romacs-agent-rpc-object "id" (romacs-agent-id session) "text" (plist-get intent :text)
                           "mode" (if (and (< minor 1) (eq (plist-get intent :kind) 'interrupt)) "next"
                                    (car (rassq (plist-get intent :kind) romacs-agent-submit-modes)))
                           "images" (vconcat (plist-get intent :attachments))
                           "draftId" (plist-get intent :draft-id)))

(defconst romacs-agent-remote-submit-deadline 30
  "Seconds a remote chat's send waits for its service's answer before it
is unconfirmed (`romacs-agent-remote--unconfirmed').  The serving editor
answers once it has kept the command and admitted the draft, in well
under a second, and seconds on a host under heavy load; a peer that is
silent past this is held up or gone.")

(cl-defmethod romacs-agent-log-source-submit ((source romacs-agent-remote) session intent callback)
  "Submit INTENT to SESSION through SOURCE's service, as `agents/send'
under its draft id, on the one client SOURCE has now: the serving
editor alone decides what it does, an interrupt's too, from protocol
1.1 (`romacs-agent-client-minor'); a 1.0 service is sent
`agents/interrupt' first.  Before the client's welcome, refused at once
\(`owner-unavailable'): sent again, it goes then.  Answered once: by the
service's reply, or, should it not come within
`romacs-agent-remote-submit-deadline', unconfirmed
\(`romacs-agent-remote--unconfirmed'); either way its client waits on no
reply of it any more.  To a 1.0 service an interrupt may go and its
send fail, and nothing keeps a draft from going twice."
  (let* ((client (romacs-agent-remote-client source))
         (minor (and client (romacs-agent-client-live-p client) (romacs-agent-client-minor client)))
         (timer nil)
         (interrupted (romacs-agent-remote--said "agents/interrupt"))
         (reply nil)
         (answer (lambda (answer)
                   (when timer
                     (cancel-timer timer)
                     (setq timer nil)
                     (romacs-agent-client-forget client reply)
                     (romacs-agent-client-forget client interrupted)
                     (romacs-agent-submit-answer callback answer)))))
    (setq reply (lambda (result error) (funcall answer (romacs-agent-remote--submit-answer intent result error minor))))
    (if (null minor)
        (romacs-agent-submit-answer
         callback (romacs-agent-submit-refused
                   intent 'owner-unavailable
                   (if (and client (romacs-agent-client-live-p client))
                       "The service is still connecting: send again"
                     (substitute-command-keys "Not connected: \\[romacs-agents-remote] connects"))))
      (setq timer (run-at-time
                   romacs-agent-remote-submit-deadline nil
                   (lambda () (funcall answer (romacs-agent-remote--unconfirmed intent minor)))))
      (condition-case err
          (progn
            (when (and (< minor 1) (eq (plist-get intent :kind) 'interrupt) (romacs-agent-log--running-p session))
              (romacs-agent-client-command client "agents/interrupt"
                                           (romacs-agent-rpc-object "id" (romacs-agent-id session)) interrupted))
            (romacs-agent-client-command client "agents/send" (romacs-agent-remote--send-params session intent minor) reply))
        (error (funcall answer (romacs-agent-submit-refused intent 'owner-unavailable (error-message-string err))))))))

(cl-defmethod romacs-agent-log-source-interrupt ((source romacs-agent-remote) session)
  (romacs-agent-remote--command source "agents/interrupt" (romacs-agent-rpc-object "id" (romacs-agent-id session))))

(defun romacs-agent-remote--answers (event chosen)
  "CHOSEN, ((QUESTION . LABELS) ...), each label as picked, as the
protocol says them for EVENT's questions: a table of question id to
{options, text}, a label not an option's its text."
  (let ((answers (make-hash-table :test #'equal)))
    (cl-loop for (text . props) in (plist-get event :questions) for q from 1
             for picked = (cdr (assoc text chosen))
             when picked
             do (let ((labels (plist-get props :options)) (options nil) (free nil))
                  (dolist (part picked)
                    (if-let* ((index (cl-position part labels :test #'equal)))
                        (push (format "o%d" (1+ index)) options)
                      (push part free)))
                  (puthash (format "q%d" q)
                           (romacs-agent-rpc-object "options" (vconcat (nreverse options))
                                                    "text" (if free (string-join (nreverse free) ", ") :null))
                           answers)))
    answers))

(cl-defmethod romacs-agent-log-source-respond ((source romacs-agent-remote) session id allow reason)
  "Answer SESSION's request ID, a handle's id, through SOURCE's service."
  (romacs-agent-remote--answer source session (gethash id (romacs-agent-approvals session)) allow reason))

(defun romacs-agent-remote--answer (source session event allow reason)
  "Answer SESSION's request EVENT asks, as it was asked, its handle whole,
through SOURCE's service."
  (let ((id (plist-get event :request-id)))
    (unless event (user-error "That request is answered"))
    (romacs-agent-remote--command
     source "agents/answer"
     (let ((params (romacs-agent-rpc-object
                    "id" (romacs-agent-id session)
                    "handle" (romacs-agent-rpc-object "run" (or (plist-get event :run) 0) "id" id)
                    "decision" (if allow "allow" "deny")
                    "scope" (if (and (consp allow) (plist-get allow :always)) "session" "once")
                    "reason" (or reason :null))))
       (when (and (consp allow) (or (plist-get allow :chosen) (plist-get allow :answers)))
         (puthash "answers" (romacs-agent-remote--answers
                             event (or (plist-get allow :chosen)
                                       ;; Answers alone: each one whole, a label or its text.
                                       (mapcar (lambda (pair) (list (car pair) (cdr pair))) (plist-get allow :answers))))
                  params))
       params))))

(cl-defmethod romacs-agent-log-source-ask ((source romacs-agent-remote) session id questions)
  "Open the layer answering SESSION's request ID, its answers sent through
SOURCE's service."
  (let ((event (gethash id (romacs-agent-approvals session))))
    ;; Its request as asked now, its run too: the layer answers that one.
    (romacs-agent-ask-open session id questions nil
                           (lambda (allow) (romacs-agent-remote--answer source session event allow nil)))))

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

(cl-defmethod romacs-agent-board-source-new ((source romacs-agent-remote) cwd)
  "Start a session on SOURCE's machine: its project read among those its
service listed, CWD offered, its provider among its instances, its first
prompt; its chat shown once the service lists it."
  (romacs-agent-remote--client)
  (let* ((projects (mapcar (lambda (p) (gethash "cwd" p)) (romacs-agent-remote-projects source)))
         (providers (mapcar (lambda (p) (gethash "name" p)) (romacs-agent-remote-providers source)))
         (project (completing-read "Project on the other machine: " projects nil nil
                                   (and cwd (directory-file-name cwd))))
         (provider (if (cdr providers) (completing-read "Provider: " providers nil t nil nil (car providers))
                     (car providers)))
         (prompt (progn
                   ;; A path on the other machine, as typed: nothing of it is
                   ;; read here.
                   (unless (and (string-prefix-p "/" project) (not (string-match-p "\\`/[^/:]+:" project)))
                     (user-error "A project on the other machine is an absolute path there: %s" project))
                   (read-string "First prompt (RET: none): "))))
    (romacs-agent-remote--lists source)
    (romacs-agent-client-command
     (romacs-agent-remote-client source) "agents/start"
     (romacs-agent-rpc-object "cwd" project "provider" (or provider :null)
                              "prompt" (if (string-empty-p prompt) :null prompt))
     (lambda (result error)
       (if error (message "Not started: %s" (gethash "message" error))
         (romacs-agent-remote--show-when-listed source (gethash "id" result) (+ (float-time) 10)))))))

(defun romacs-agent-remote--show-when-listed (source id deadline)
  "Show the chat of SOURCE's session ID once its service lists it, until DEADLINE."
  (cond ((not (eq source romacs-agent-remote--source)))
        ((gethash id (romacs-agent-remote--rows source))
         (pop-to-buffer (romacs-agent-remote-chat id)))
        ((< (float-time) deadline)
         (run-at-time 0.2 nil #'romacs-agent-remote--show-when-listed source id deadline))
        (t (message "Started %s; not listed yet" id))))

;;; Connecting

(defun romacs-agent-remote--socket ()
  "The local socket `romacs-relay connect' listens on: in a directory of
its own, which the relay makes the user's alone (the editor makes
`romacs-agent--runtime-root' 0755)."
  (expand-file-name "remote/remote.sock" (romacs-agent--runtime-root)))

(defun romacs-agent-remote-connect (&optional socket)
  "Show another machine's agent sessions: connect to the service whose
SOCKET is given, else start `romacs-relay connect' to
`romacs-agent-remote-host' and connect through it once it listens, the
editor not waiting.  Connected again, the chats it shows are followed
afresh.  The source."
  (let ((source (or romacs-agent-remote--source (romacs-agent-remote--make))))
    (setq romacs-agent-remote--source source romacs-agent-board-source source)
    (when (romacs-agent-remote-client source) (romacs-agent-client-close (romacs-agent-remote-client source)))
    (setf (romacs-agent-remote-client source) nil (romacs-agent-remote-dial source) nil)
    (if socket
        (romacs-agent-remote--connected source (romacs-agent-client-connect socket))
      (unless romacs-agent-remote-host (user-error "No machine named: set `romacs-agent-remote-host'"))
      (unless (process-live-p (romacs-agent-remote-relay source))
        ;; Its socket's directory the relay's to make: the user's alone.
        (setf (romacs-agent-remote-relay source)
              (make-process :name "romacs-relay" :buffer (generate-new-buffer " *romacs-relay*") :noquery t
                            :command (list romacs-agent-remote-relay "connect" romacs-agent-remote-host
                                           "--socket" (romacs-agent-remote--socket)))))
      (let ((dial (list 'dial)))
        (setf (romacs-agent-remote-dial source) dial)
        (romacs-agent-remote--dial source dial (+ (float-time) 10))))
    source))

(defun romacs-agent-remote--dial (source dial deadline)
  "Connect SOURCE through its relay once it listens, trying again a moment
later until DEADLINE, while DIAL is its attempt: a socket there before
it, a stale one, refuses until the relay replaces it."
  (let ((relay (romacs-agent-remote-relay source)))
    (when (and (eq source romacs-agent-remote--source) (eq dial (romacs-agent-remote-dial source)))
      (if-let* ((client (ignore-error file-error (romacs-agent-client-connect (romacs-agent-remote--socket)))))
          (progn (setf (romacs-agent-remote-dial source) nil)
                 (romacs-agent-remote--connected source client))
        (if (and (process-live-p relay) (< (float-time) deadline))
            (run-at-time 0.05 nil #'romacs-agent-remote--dial source dial deadline)
          (when (process-live-p relay) (delete-process relay))
          (message "No relay to %s: %s" romacs-agent-remote-host
                   (with-current-buffer (process-buffer relay) (string-trim (buffer-string)))))))))

(defun romacs-agent-remote--lists (source)
  "Ask SOURCE's service what a session may start in and under, kept for
the next start."
  (let ((client (romacs-agent-remote-client source)))
    (romacs-agent-client-request client "agents/projects.list" (romacs-agent-rpc-object)
                                 (lambda (result _) (when result (setf (romacs-agent-remote-projects source)
                                                                        (append (gethash "projects" result) nil)))))
    (romacs-agent-client-request client "agents/providers.list" (romacs-agent-rpc-object)
                                 (lambda (result _) (when result (setf (romacs-agent-remote-providers source)
                                                                        (append (gethash "providers" result) nil)))))))

(defun romacs-agent-remote--connected (source client)
  "SOURCE connected through CLIENT: each chat it shows followed through it."
  (setf (romacs-agent-remote-client source) client)
  (romacs-agent-remote--lists source)
  (maphash (lambda (id buffer)
             (when (buffer-live-p buffer)
               (romacs-agent-client-follow client id romacs-agent-log-turns romacs-agent-log-records)))
           (romacs-agent-remote-chats source)))

(defun romacs-agent-remote-disconnect ()
  "Stop showing another machine's sessions."
  (when-let* ((source romacs-agent-remote--source))
    (setq romacs-agent-remote--source nil romacs-agent-board-source romacs-agent-log-local)
    (when (romacs-agent-remote-client source) (romacs-agent-client-close (romacs-agent-remote-client source)))
    (when (process-live-p (romacs-agent-remote-relay source)) (delete-process (romacs-agent-remote-relay source)))
    ;; The board shows this editor's own again.
    (romacs-agent-board--refresh)))

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
