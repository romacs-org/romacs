;;; romacs-agent-remote.el --- Another machine's agent sessions, through its service  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Romacs on other machines (doc/agent-protocol.org, Clients and
;; Transports): the agent sessions of the machines
;; `romacs-agent-remote-hosts' names, drawn here as this editor draws its
;; own.  An ssh pipe (`ssh HOST romacs-relay stdio') or `romacs-relay
;; connect' to a relay's URL carries the protocol from a machine's service
;; here; a client (romacs-agent-client.el) mirrors what the service
;; serves; and each machine is a chat's source (`romacs-agent-log--source')
;; made of its client, so what is kept of a session (its mirror, its chat)
;; is kept by machine and id together.  A connection that ends is made
;; again, a little later each time, until the machine is let go.
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

(defcustom romacs-agent-remote-hosts nil
  "The other machines whose agent sessions this editor shows, each
\\(NAME . PLACE), NAME how it is called here.  PLACE is either
\\(:ssh DESTINATION :relay COMMAND), the machine reached over ssh, where
DESTINATION is what ssh takes (a host, USER@HOST, a Host of its config)
and COMMAND runs `romacs-relay' in the remote shell, which is no login
shell (`romacs-agent-remote-relay-command' when it is left out); or
\\(:url URL), the machine's relay URL (wss://HOST/agents), reached through
`romacs-relay connect'."
  :type '(alist :key-type (string :tag "Name")
                :value-type (choice (list :tag "SSH" (const :ssh) (string :tag "Destination")
                                          (const :relay) (string :tag "Remote romacs-relay"))
                                    (list :tag "Relay URL" (const :url) string)))
  :group 'romacs-agent-remote)

(defcustom romacs-agent-remote-host nil
  "A machine's relay URL, as `romacs-relay connect' takes it
\\(wss://HOST/agents), or nil: a host of `romacs-agent-remote-hosts' named
by its URL."
  :type '(choice (const :tag "None" nil) string) :group 'romacs-agent-remote)
(make-obsolete-variable 'romacs-agent-remote-host 'romacs-agent-remote-hosts "0.0.22")

(defcustom romacs-agent-remote-relay "romacs-relay"
  "The `romacs-relay' program here, which `romacs-relay connect' runs."
  :type 'string :group 'romacs-agent-remote)

(defcustom romacs-agent-remote-relay-command "romacs-relay"
  "How the remote shell runs `romacs-relay' over ssh, for a host that names
no command of its own: an absolute path, or one from the remote home
\\(as ~/PATH), since ssh's remote shell is no login shell."
  :type 'string :group 'romacs-agent-remote)

(defcustom romacs-agent-remote-ssh "ssh"
  "The ssh program."
  :type 'string :group 'romacs-agent-remote)

(defconst romacs-agent-remote--ssh-options
  '("-T" "-x" "-o" "BatchMode=yes" "-o" "ServerAliveInterval=15" "-o" "ServerAliveCountMax=3"
    "-o" "RemoteCommand=none")
  "ssh's options for a pipe to a service: no pty and no X; never a prompt;
a dead link ended within a minute; the relay run whatever command the
host's ssh configuration names.")

(defconst romacs-agent-remote--master-options '("-o" "ControlMaster=auto" "-o" "ControlPersist=600")
  "ssh's options for a master connection kept ten minutes, so connecting
again costs a channel, not a handshake.")

(defconst romacs-agent-remote--socket-path-most 107
  "The most bytes of a Unix socket's file name.")

(defconst romacs-agent-remote--retry-first 0.25
  "The seconds before a machine whose connection ended is connected again
the first time: a master connection makes it a channel's cost.")

(defconst romacs-agent-remote--retry-most 30
  "The most seconds between attempts to connect a machine again.")

(cl-defstruct (romacs-agent-remote (:constructor romacs-agent-remote--make) (:copier nil))
  "Another machine's sessions as a chat's source: its NAME and PLACE
\\(`romacs-agent-remote-hosts'; a socket's file name for one reached
directly), the CLIENT of its service, the RELAY process that carries it
here, the mirror of each of its sessions, by id (SESSIONS), their CHATS by
id, while it connects the DIAL its one attempt is named by, the PROJECTS
and PROVIDERS its service last listed (`agents/projects.list',
`agents/providers.list'), and, while it is to be connected again, the
RETRY timer and the seconds it waits (DELAY)."
  name place client relay dial projects providers retry (delay romacs-agent-remote--retry-first)
  (sessions (make-hash-table :test #'equal))
  (chats (make-hash-table :test #'equal)))

(defvar romacs-agent-remote--sources (make-hash-table :test #'equal)
  "Each machine this editor is connected to, by name: its source.")

(defvar romacs-agent-remote--source nil
  "The other machine whose sessions the board shows, or nil.")

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

(defun romacs-agent-remote--of (client)
  "The machine whose service CLIENT is connected to, or nil."
  (if (and romacs-agent-remote--source (eq client (romacs-agent-remote-client romacs-agent-remote--source)))
      romacs-agent-remote--source
    (cl-find client (hash-table-values romacs-agent-remote--sources) :key #'romacs-agent-remote-client)))

(defun romacs-agent-remote--followed (client id notice)
  "What came of following session ID on CLIENT, in its chat and board."
  (when-let* ((source (romacs-agent-remote--of client)))
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
                                            "the connection ended: connecting again"))))))))))

(add-hook 'romacs-agent-client-follow-functions #'romacs-agent-remote--followed)

(cl-defmethod romacs-agent-log-source-fill ((source romacs-agent-remote) session)
  "Follow SESSION on SOURCE's service, drawing events as they arrive."
  (romacs-agent-client-follow (romacs-agent-remote-client source) (romacs-agent-id session)
                              romacs-agent-log-turns romacs-agent-log-records))

(cl-defmethod romacs-agent-log-source-close ((source romacs-agent-remote) id)
  "Stop following session ID on SOURCE's service."
  (when-let* ((client (romacs-agent-remote-client source)))
    (romacs-agent-client-unfollow client id)))

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
      (setf (romacs-agent-approvals session) approvals)
      (romacs-agent--summary-stale id))))

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

(cl-defmethod romacs-agent-summary--entries ((source romacs-agent-remote) id)
  "SOURCE's service's row of ID, or each row it lists, stamping its
summary: a row changed comes as a new one.  Its record is its mirror;
its key is the service's, whatever its parent here."
  (let ((rows (romacs-agent-remote--rows source)))
    (cl-flet ((entry (row) (cl-list* (gethash "id" row) row nil (lambda () (romacs-agent-remote--mirror source row)))))
      (if id
          (and-let* ((row (gethash id rows))) (list (entry row)))
        (let ((entries nil))
          (maphash (lambda (_ row) (push (entry row) entries)) rows)
          entries)))))

(cl-defmethod romacs-agent-summary--key ((source romacs-agent-remote) session _known)
  "The key SOURCE's service made for SESSION: drawn, never derived here."
  (let ((key (gethash "key" (gethash (romacs-agent-id session) (romacs-agent-remote--rows source)))))
    (if (stringp key) (intern key) 'idle)))

(cl-defmethod romacs-agent-board-source-live ((source romacs-agent-remote) id)
  "The mirror of SOURCE's session ID, while its service lists it."
  (when-let* ((row (gethash id (romacs-agent-remote--rows source))))
    (romacs-agent-remote--mirror source row)))

(defun romacs-agent-remote--rows-changed (client _changed _removed)
  "CLIENT's list changed: its machine answers, so a connection that ends
is tried again soon; the board shows it again when it shows that machine."
  (when-let* ((source (romacs-agent-remote--of client)))
    (setf (romacs-agent-remote-delay source) romacs-agent-remote--retry-first)
    (when (eq source romacs-agent-remote--source) (romacs-agent-board--refresh))))

(add-hook 'romacs-agent-client-rows-functions #'romacs-agent-remote--rows-changed)

(cl-defmethod romacs-agent-board-source-new ((source romacs-agent-remote) cwd)
  "Start a session on SOURCE's machine: its project read among those its
service listed, CWD offered, its provider among its instances, its first
prompt; its chat shown once the service lists it."
  (romacs-agent-remote--live-client source)
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
  (cond ((not (eq source (gethash (romacs-agent-remote-name source) romacs-agent-remote--sources))))
        ((gethash id (romacs-agent-remote--rows source))
         (pop-to-buffer (romacs-agent-remote-chat id source)))
        ((< (float-time) deadline)
         (run-at-time 0.2 nil #'romacs-agent-remote--show-when-listed source id deadline))
        (t (message "Started %s; not listed yet" id))))

;;; Connecting

(defun romacs-agent-remote--hosts ()
  "Every machine named, each (NAME . PLACE): `romacs-agent-remote-hosts',
then `romacs-agent-remote-host' as a machine named by its URL."
  (append romacs-agent-remote-hosts
          (and romacs-agent-remote-host (list (list romacs-agent-remote-host :url romacs-agent-remote-host)))))

(defun romacs-agent-remote--socket (name)
  "The local socket `romacs-relay connect' listens on for the machine NAME:
in a directory of its own, which the relay makes the user's alone (the
editor makes `romacs-agent--runtime-root' 0755)."
  (expand-file-name (format "remote/%s.sock" (substring (secure-hash 'sha256 name) 0 16))
                    (romacs-agent--runtime-root)))

(defun romacs-agent-remote--control-dir ()
  "The directory of ssh's master connections' sockets, made the user's
alone, in the agents' runtime directory; nil when a socket there would
not fit a socket's name (ssh's digest, 40 bytes, and the suffix of its
temporary name, 17, after it), or when ssh would read its name otherwise
\(a space, a quote, a backslash, a % or a $)."
  (let ((dir (expand-file-name "ssh" (romacs-agent--runtime-root))))
    (when (and (<= (+ (string-bytes dir) 1 40 17) romacs-agent-remote--socket-path-most)
               (not (string-match-p "[[:space:]%\"'$\\]" dir)))
      (make-directory dir t)
      (set-file-modes dir #o700)
      dir)))

(defun romacs-agent-remote--ssh-command (place control)
  "The command whose pipes carry the protocol to the service of PLACE, a
machine reached over ssh, its master connection's socket in the directory
CONTROL, named by ssh's digest of the destination (%C); with no CONTROL,
no master."
  `(,romacs-agent-remote-ssh ,@romacs-agent-remote--ssh-options
                             ,@(if control
                                   `(,@romacs-agent-remote--master-options
                                     "-o" ,(concat "ControlPath=" (expand-file-name "%C" control)))
                                 '("-o" "ControlMaster=no" "-o" "ControlPath=none"))
                             ,(plist-get place :ssh)
                             ,(or (plist-get place :relay) romacs-agent-remote-relay-command) "stdio"))

(defun romacs-agent-remote--stderr (source)
  "The buffer of what SOURCE's ssh says on its standard error, emptied."
  (with-current-buffer (get-buffer-create (format " *romacs-agent ssh %s*" (romacs-agent-remote-name source)))
    (erase-buffer)
    (current-buffer)))

(defun romacs-agent-remote--said-last (source)
  "The last line SOURCE's ssh said on its standard error, or nil."
  (when-let* ((buffer (get-buffer (format " *romacs-agent ssh %s*" (romacs-agent-remote-name source)))))
    (with-current-buffer buffer
      (car (last (split-string (buffer-string) "[\r\n]+" t "[ \t]+"))))))

(defun romacs-agent-remote--host-source (name place)
  "The source of the machine NAME at PLACE: the one this editor keeps for
NAME while it is at PLACE, else a new one, kept.  One kept for NAME at
another place is another machine's: let go of, its relay with it."
  (let ((kept (gethash name romacs-agent-remote--sources)))
    (if (and kept (equal (romacs-agent-remote-place kept) place))
        kept
      (when kept (romacs-agent-remote-disconnect kept))
      ;; Its own copy, strings too: PLACE edited where it came from is
      ;; another place.
      (puthash name (romacs-agent-remote--make
                     :name name :place (if (stringp place) (copy-sequence place)
                                         (mapcar (lambda (x) (if (stringp x) (copy-sequence x) x)) place)))
               romacs-agent-remote--sources))))

(defun romacs-agent-remote-connect (&optional socket host)
  "Show another machine's agent sessions on the board: HOST, a name of
`romacs-agent-remote-hosts' (the first by default), or, given SOCKET, the
service whose socket it is, named HOST, else by its file name.  Its
service is connected afresh, the editor not waiting, and each chat of it
shown is followed afresh once it is.  Machines connected before stay so.
The source."
  (let* ((name (or host socket (car (car (romacs-agent-remote--hosts)))
                   (user-error "No machine named: set `romacs-agent-remote-hosts'")))
         (place (or socket (cdr (assoc name (romacs-agent-remote--hosts)))
                    (user-error "No machine %s in `romacs-agent-remote-hosts'" name)))
         (source (romacs-agent-remote--host-source name place)))
    (setq romacs-agent-remote--source source romacs-agent-board-source source)
    (romacs-agent-remote--open source)
    source))

(defun romacs-agent-remote--open (source)
  "Connect SOURCE to its machine's service afresh, by its place: a socket,
an ssh pipe, or a relay URL through `romacs-relay connect'.  A connection
that cannot be made, or that ends as it is made, is one that ended: it is
tried again (`romacs-agent-remote--again'), what failed said."
  (when (timerp (romacs-agent-remote-retry source)) (cancel-timer (romacs-agent-remote-retry source)))
  (let ((old (romacs-agent-remote-client source)) (place (romacs-agent-remote-place source)))
    ;; Not SOURCE's once closed: its end is no reason to connect again.
    (setf (romacs-agent-remote-client source) nil (romacs-agent-remote-dial source) nil
          (romacs-agent-remote-retry source) nil)
    (when old (romacs-agent-client-close old))
    (condition-case err
        (cond
         ((stringp place) (romacs-agent-remote--connected source (romacs-agent-client-connect place)))
         ((plist-get place :ssh)
          (romacs-agent-remote--connected
           source (romacs-agent-client-connect
                   (romacs-agent-remote--ssh-command place (romacs-agent-remote--control-dir))
                   (romacs-agent-remote--stderr source))))
         (t (romacs-agent-remote--relay source (plist-get place :url))))
      (error (message "Agents on %s: not connected: %s" (romacs-agent-remote-name source) (error-message-string err))
             (romacs-agent-remote--again source)))))

(defun romacs-agent-remote--relay (source url)
  "Connect SOURCE through `romacs-relay connect' to URL, started unless it
runs, once it listens; the editor not waiting."
  (let ((socket (romacs-agent-remote--socket (romacs-agent-remote-name source))))
    (unless (process-live-p (romacs-agent-remote-relay source))
      ;; Its socket's directory the relay's to make: the user's alone.
      (setf (romacs-agent-remote-relay source)
            (make-process :name "romacs-relay" :noquery t
                          :buffer (with-current-buffer (get-buffer-create (format " *romacs-relay %s*" (romacs-agent-remote-name source)))
                                    (erase-buffer)
                                    (current-buffer))
                          :command (list romacs-agent-remote-relay "connect" url "--socket" socket))))
    (let ((dial (list 'dial)))
      (setf (romacs-agent-remote-dial source) dial)
      (romacs-agent-remote--dial source dial (+ (float-time) 10)))))

(defun romacs-agent-remote--dial (source dial deadline)
  "Connect SOURCE through its relay once it listens, trying again a moment
later until DEADLINE, while DIAL is its attempt: a socket there before
it, a stale one, refuses until the relay replaces it."
  (let ((relay (romacs-agent-remote-relay source)))
    (when (eq dial (romacs-agent-remote-dial source))
      (if-let* ((client (ignore-error file-error
                          (romacs-agent-client-connect (romacs-agent-remote--socket (romacs-agent-remote-name source))))))
          (progn (setf (romacs-agent-remote-dial source) nil)
                 (romacs-agent-remote--connected source client))
        (if (and (process-live-p relay) (< (float-time) deadline))
            (run-at-time 0.05 nil #'romacs-agent-remote--dial source dial deadline)
          (when (process-live-p relay) (delete-process relay))
          (message "No relay to %s: %s" (romacs-agent-remote-name source)
                   (with-current-buffer (process-buffer relay) (string-trim (buffer-string))))
          (romacs-agent-remote--again source))))))

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

(defconst romacs-agent-remote--answer-within 20
  "Seconds a machine's service has to answer: to list its sessions once
connected, then each check after (`romacs-agent-remote--check'); past
them the connection is closed and made again.")

(defun romacs-agent-remote--connected (source client)
  "SOURCE connected through CLIENT: each chat it shows followed through it.
A CLIENT ended already, or whose service stops answering
\(`romacs-agent-remote--check'), is one that ended."
  (setf (romacs-agent-remote-client source) client)
  (if (not (romacs-agent-client-live-p client))
      (romacs-agent-remote--again source)
    (romacs-agent-remote--lists source)
    (maphash (lambda (id buffer)
               (when (buffer-live-p buffer)
                 (romacs-agent-client-follow client id romacs-agent-log-turns romacs-agent-log-records)))
             (romacs-agent-remote-chats source))
    (romacs-agent-remote--check client source nil)))

(defun romacs-agent-remote--check (client source asked)
  "Close CLIENT, SOURCE's, when its service has not answered: its list not
watched, or ASKED, a check's (ANSWERED), still unanswered, after
`romacs-agent-remote--answer-within' seconds; else ask a request any
service answers at once (an unknown method's refusal answers it too),
and check again that long later, for as long as CLIENT is SOURCE's.  A
live ssh link does not make a live service."
  (run-at-time romacs-agent-remote--answer-within nil
               (lambda ()
                 (when (and (eq client (romacs-agent-remote-client source)) (romacs-agent-client-live-p client))
                   (if (or (not (romacs-agent-client-revision client)) (and asked (not (car asked))))
                       (romacs-agent-client-close client)
                     (let ((answered (list nil)))
                       (romacs-agent-client-request client "agents/ping" (romacs-agent-rpc-object)
                                                    (lambda (_result _error) (setcar answered t)))
                       (romacs-agent-remote--check client source answered)))))))

(defun romacs-agent-remote--again (source)
  "Connect SOURCE again after its delay, doubled for the next time up to
`romacs-agent-remote--retry-most' seconds, while this editor keeps it;
one attempt waits at a time."
  (when (timerp (romacs-agent-remote-retry source)) (cancel-timer (romacs-agent-remote-retry source)))
  (let ((delay (romacs-agent-remote-delay source)))
    (setf (romacs-agent-remote-delay source) (min romacs-agent-remote--retry-most (* 2 delay))
          (romacs-agent-remote-retry source)
          (run-at-time delay nil (lambda ()
                                   (when (eq source (gethash (romacs-agent-remote-name source) romacs-agent-remote--sources))
                                     (romacs-agent-remote--open source)))))))

(defun romacs-agent-remote--ended (client)
  "CLIENT's connection ended: its machine, while this editor keeps it, is
connected again a little later (`romacs-agent-remote--again'), and what
its ssh said last is told."
  (when-let* ((source (romacs-agent-remote--of client)))
    (message "Agents on %s: the connection ended%s; connecting again in %g s"
             (romacs-agent-remote-name source)
             (if-let* ((said (romacs-agent-remote--said-last source))) (format " (%s)" said) "")
             (romacs-agent-remote-delay source))
    (romacs-agent-remote--again source)))

(add-hook 'romacs-agent-client-ended-functions #'romacs-agent-remote--ended)

(defun romacs-agent-remote-disconnect (&optional source)
  "Let go of SOURCE, another machine, the one the board shows by default:
its connection closed, its relay stopped, connected no more; the board
shows this editor's own sessions again when it showed SOURCE's."
  (when-let* ((source (or source romacs-agent-remote--source)))
    ;; No longer kept: its connection's end is no reason to connect again.
    ;; Another kept under its name since is not it.
    (when (eq source (gethash (romacs-agent-remote-name source) romacs-agent-remote--sources))
      (remhash (romacs-agent-remote-name source) romacs-agent-remote--sources))
    (when (timerp (romacs-agent-remote-retry source)) (cancel-timer (romacs-agent-remote-retry source)))
    (when (eq source romacs-agent-remote--source)
      (setq romacs-agent-remote--source nil romacs-agent-board-source romacs-agent-log-local))
    ;; Nor does a dial under way connect it.
    (setf (romacs-agent-remote-dial source) nil)
    (when (romacs-agent-remote-client source) (romacs-agent-client-close (romacs-agent-remote-client source)))
    (when (process-live-p (romacs-agent-remote-relay source)) (delete-process (romacs-agent-remote-relay source)))
    (romacs-agent-board--refresh)))

(defun romacs-agent-remote--live-client (source)
  "SOURCE's client, connected, or a user error."
  (let ((client (and source (romacs-agent-remote-client source))))
    (unless (and client (romacs-agent-client-live-p client))
      (user-error (substitute-command-keys "Not connected: \\[romacs-agents-remote] connects")))
    client))

(defun romacs-agent-remote--read-host ()
  "A machine's name of `romacs-agent-remote-hosts', read when there are
several, the board's own offered first."
  (let ((names (mapcar #'car (romacs-agent-remote--hosts))))
    (cond ((null names) (user-error "No machine named: set `romacs-agent-remote-hosts'"))
          ((null (cdr names)) (car names))
          (t (completing-read "Agents on: " names nil t nil nil
                              (and romacs-agent-remote--source (romacs-agent-remote-name romacs-agent-remote--source)))))))

(defun romacs-agents-remote (host)
  "Show the agent board of HOST, a machine of `romacs-agent-remote-hosts',
read when there are several: its sessions, drawn from its agents
service, connected unless it is.  Machines connected before stay so."
  (interactive (list (romacs-agent-remote--read-host)))
  (let* ((source (gethash host romacs-agent-remote--sources))
         (client (and source (romacs-agent-remote-client source))))
    ;; Connected where HOST is now named: shown; else connected there.
    (if (and client (romacs-agent-client-live-p client)
             (equal (romacs-agent-remote-place source) (cdr (assoc host (romacs-agent-remote--hosts)))))
        (setq romacs-agent-remote--source source romacs-agent-board-source source)
      (romacs-agent-remote-connect nil host)))
  (romacs-agents))

(defun romacs-agent-remote-chat (id &optional source)
  "The chat of session ID of SOURCE, another machine (the one the board
shows by default), drawn from its service."
  (let* ((source (or source romacs-agent-remote--source))
         (client (romacs-agent-remote--live-client source))
         (row (or (gethash id (romacs-agent-client-rows client)) (user-error "No session %s there" id))))
    (romacs-agent-log-chat (romacs-agent-remote--mirror source row) nil source)))

(provide 'romacs-agent-remote)
;;; romacs-agent-remote.el ends here
