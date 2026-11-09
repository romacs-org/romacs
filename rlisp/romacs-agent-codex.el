;;; romacs-agent-codex.el --- Codex, as Romacs runs it  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Codex's driver (`romacs-agent-define-harness'): one `codex app-server'
;; process per session, JSON-RPC lines both ways (codex-cli 0.164).  It
;; starts up by `initialize', `initialized', then `thread/start', or
;; `thread/resume' going on with its thread (one `codex exec' made too).
;; A message is a `turn/start', or a `turn/steer' into the turn running;
;; an interrupt is `turn/interrupt'.  Its approval requests come to the
;; user; any other request it makes is refused.
;;
;; A request's id names its method ("turn/start:12"): an answer says what
;; it answers with no table kept.  Sessions an earlier Romacs ran by
;; `codex exec --json' journaled that run's lines: they read as before.

;;; Code:

(require 'romacs-agent)

(declare-function romacs-agent--instructions "romacs-agent" (session))
(declare-function romacs-agent--image-caption "romacs-agent" (file))

;;; Writing to it

(defun romacs-agent--codex-request (method number params)
  "The request METHOD with PARAMS, its id NUMBER under METHOD's name."
  (romacs-mcp--object "id" (format "%s:%d" method number) "method" method "params" params))

(defun romacs-agent--codex-method (id)
  "The method a request id of ours names, or nil for another id."
  (and (stringp id) (string-match "\\`\\([^:]+\\):[0-9]+\\'" id) (match-string 1 id)))

(defun romacs-agent--codex-mcp-words (session)
  (list "-c" (format "mcp_servers.romacs.command=%S" (romacs-agent--program "romacs-mcp"))
        "-c" (format "mcp_servers.romacs.args=[%s]"
                     (mapconcat (lambda (word) (format "%S" word))
                                (list "--socket" (romacs-agent--route session) "--reconnect"
                                      "--session" (romacs-agent-id session)
                                      "--reply-timeout" (number-to-string romacs-agent-reply-timeout))
                                ","))))

(defun romacs-agent--codex-argv (session _resume)
  "`codex app-server''s command line for SESSION: whether it goes on with
a thread is its startup's to say."
  (let ((instance (romacs-agent--instance (romacs-agent-provider session) 'codex)))
    (append (romacs-agent--environment-words session)
            (list (plist-get instance :program) "app-server")
            ;; Romacs's tools, through the reconnecting relay.
            (romacs-agent--codex-mcp-words session)
            (plist-get instance :arguments))))

(defun romacs-agent--codex-thread (session number)
  "The request starting SESSION's thread, or going on with the one it has."
  (let ((thread (romacs-agent-harness-session session))
        (params (romacs-mcp--object "cwd" (romacs-agent-cwd session)
                                    "developerInstructions" (romacs-agent--instructions session))))
    (when-let* ((model (plist-get (romacs-agent-options session) :model))) (puthash "model" model params))
    (if thread
        (progn (puthash "threadId" thread params)
               ;; Its turns are in the journal already.
               (puthash "excludeTurns" t params)
               (romacs-agent--codex-request "thread/resume" number params))
      (romacs-agent--codex-request "thread/start" number params))))

(defun romacs-agent--codex-text (text)
  (romacs-mcp--object "type" "text" "text" text "text_elements" []))

(defun romacs-agent--codex-items (text)
  "The input items of a message of TEXT, with the images attached to it,
each by its path, named in a caption before it."
  (vconcat (list (romacs-agent--codex-text text))
           (mapcan (lambda (file)
                     (list (romacs-agent--codex-text (romacs-agent--image-caption file))
                           (romacs-mcp--object "type" "localImage" "path" file)))
                   (seq-filter #'romacs-agent-image-file-p romacs-agent--message-images))))

(defun romacs-agent--codex-turn (session number items)
  "The `turn/start' of ITEMS, on SESSION's model and effort, its reasoning
summarized in detail."
  (let ((params (romacs-mcp--object "threadId" (romacs-agent-harness-session session) "input" items
                                    "summary" "detailed"))
        (options (romacs-agent-options session)))
    (when-let* ((model (plist-get options :model))) (puthash "model" model params))
    (when-let* ((effort (plist-get options :effort))) (puthash "effort" effort params))
    (romacs-agent--codex-request "turn/start" number params)))

(defun romacs-agent--codex-input (session text id)
  "The message of TEXT: steering the turn running, else a new turn."
  (let ((items (romacs-agent--codex-items text))
        (turn (plist-get (romacs-agent-driver-state session) :turn)))
    (if turn
        (romacs-agent--codex-request "turn/steer" id
                                     (romacs-mcp--object "threadId" (romacs-agent-harness-session session)
                                                         "expectedTurnId" turn "input" items))
      (romacs-agent--codex-turn session id items))))

(defun romacs-agent--codex-interrupt (session)
  (when-let* ((turn (plist-get (romacs-agent-driver-state session) :turn)))
    (romacs-agent--codex-request "turn/interrupt" (romacs-agent-request-id session)
                                 (romacs-mcp--object "threadId" (romacs-agent-harness-session session)
                                                     "turnId" turn))))

(defun romacs-agent--codex-respond (session request-id allow _message)
  "The answer to approval REQUEST-ID: what it asked when ALLOW, else
declined (Codex takes no reason)."
  (pcase-let ((`(,method . ,params) (gethash request-id (romacs-agent-approvals session))))
    (romacs-mcp--object
     "id" request-id
     "result" (if (equal method "item/permissions/requestApproval")
                  (romacs-mcp--object "permissions" (if allow (gethash "permissions" params) (romacs-mcp--object))
                                      "scope" "turn")
                (romacs-mcp--object "decision" (if allow "accept" "decline"))))))

(defun romacs-agent--codex-select (session model effort)
  "Nothing to write: each turn names its model and effort.  Neither can be
cleared in a live thread: `restart' then."
  (let ((now (romacs-agent--selection session)))
    (when (or (and (plist-get now :model) (not model)) (and (plist-get now :effort) (not effort)))
      'restart)))

;;; Its startup, and what it is owed

(defun romacs-agent--codex-owed-line (session owed)
  "The line OWED stands for: a refusal of a request Romacs does not
answer, or a message a steer missed its turn with, as a new turn."
  (pcase owed
    (`(:reply ,id ,method)
     (romacs-mcp--object "id" id "error" (romacs-mcp--object "code" -32601
                                                             "message" (format "Romacs does not answer %s" method))))
    (`(:turn ,steer ,text ,images)
     (let ((line (romacs-agent--codex-turn session (romacs-agent-request-id session)
                                           (let ((romacs-agent--message-images images))
                                             (romacs-agent--codex-items text)))))
       ;; Its echo is known by the steer it stands for.
       (puthash "clientUserMessageId" steer (gethash "params" line))
       line))))

(defun romacs-agent--codex-next (session)
  "SESSION's next line from this run's state: what it owes, else its
startup's next step."
  (let* ((state (romacs-agent-driver-state session))
         (sent (plist-get state :sent)))
    (cond ((plist-get state :owed) (romacs-agent--codex-owed-line session (car (plist-get state :owed))))
          ((plist-get state :thread) :ready)
          ((null sent)
           (romacs-agent--codex-request
            "initialize" (romacs-agent-request-id session)
            (romacs-mcp--object "clientInfo" (romacs-mcp--object "name" "romacs" "title" "Romacs"
                                                                 "version" emacs-version)
                                "capabilities" (romacs-mcp--object "experimentalApi" :false "requestAttestation" :false
                                                                   "optOutNotificationMethods" ["turn/diff/updated"]))))
          ((and (equal sent "initialize") (plist-get state :initialized))
           (romacs-mcp--object "method" "initialized"))
          ((equal sent "initialized") (romacs-agent--codex-thread session (romacs-agent-request-id session))))))

(defun romacs-agent--codex-paid (session test)
  "The handshake event dropping SESSION's first owed line TEST accepts."
  (let ((owed (plist-get (romacs-agent-driver-state session) :owed)))
    (when-let* ((done (cl-find-if test owed)))
      (list (list :type 'handshake :state (list :owed (remove done owed)))))))

(defun romacs-agent--codex-input-text (items)
  (mapconcat (lambda (item) (if (equal (gethash "type" item) "text") (gethash "text" item) "")) items ""))

(defun romacs-agent--codex-message (session object)
  "The events of OBJECT, a message written: its input, and for a steer
what to do should it miss its turn."
  (let* ((params (gethash "params" object))
         (text (romacs-agent--codex-input-text (gethash "input" params)))
         (images (append (or (gethash "images" object)
                             (cl-loop for item across (gethash "input" params)
                                      when (equal (gethash "type" item) "localImage") collect (gethash "path" item)))
                         nil))
         (steer (romacs-agent--value (gethash "clientUserMessageId" params))))
    (append (list (list :type 'input :text text :images images))
            (if (equal (gethash "method" object) "turn/steer")
                (list (list :type 'handshake
                            :state (list :steers (cons (list (gethash "id" object) text images)
                                                       (plist-get (romacs-agent-driver-state session) :steers)))))
              (and steer (romacs-agent--codex-paid
                          session (lambda (owed) (and (eq (car owed) :turn) (equal (cadr owed) steer)))))))))

(defun romacs-agent--codex-input-events (session object)
  "The events of OBJECT, written to SESSION's harness."
  (let ((method (gethash "method" object)) (id (gethash "id" object)))
    (cond
     ((member method '("turn/start" "turn/steer")) (romacs-agent--codex-message session object))
     ((member method '("initialize" "initialized" "thread/start" "thread/resume"))
      (list (list :type 'handshake :state (list :sent (if (string-prefix-p "thread/" method) "thread" method)))))
     (method nil)
     ((gethash id (romacs-agent-approvals session))
      (remhash id (romacs-agent-approvals session))
      (let ((result (gethash "result" object)))
        (list (list :type 'answered :request-id id
                    :behavior (if (or (member (gethash "decision" result) '("accept" "acceptForSession"))
                                      (let ((granted (gethash "permissions" result)))
                                        (and (hash-table-p granted) (> (hash-table-count granted) 0))))
                                  "allow" "deny")))))
     (t (romacs-agent--codex-paid session (lambda (owed) (and (eq (car owed) :reply) (equal (cadr owed) id))))))))

;;; What it writes

(defun romacs-agent--codex-error (object)
  "The message of OBJECT's error, or nil."
  (let ((error (romacs-agent--value (gethash "error" object))))
    (cond ((hash-table-p error) (or (romacs-agent--value (gethash "message" error)) "error"))
          ((stringp error) error))))

(defun romacs-agent--codex-answer (session id object)
  "The events of OBJECT, the answer to our request ID."
  (let ((result (romacs-agent--value (gethash "result" object)))
        (error (romacs-agent--codex-error object))
        (method (romacs-agent--codex-method id)))
    (pcase method
      ((guard (and error (member method '("initialize" "thread/start" "thread/resume"))))
       (list (list :type 'error :text (format "Codex could not start: %s" error))))
      ("initialize" (list (list :type 'handshake :state (list :initialized t))))
      ((or "thread/start" "thread/resume")
       (let ((thread (gethash "id" (gethash "thread" result))))
         (append (list (list :type 'handshake :state (list :thread thread))
                       (list :type 'session :harness-session thread
                             :model (or (romacs-agent--value (gethash "model" result))
                                        (plist-get (romacs-agent-options session) :model))))
                 ;; Codex takes up no turn its last run was cut in: it is over.
                 (when (romacs-agent-interrupted session)
                   (list (list :type 'done :error t :reason "Cut short when Codex stopped"))))))
      ("turn/start"
       (if error
           (list (list :type 'done :error t :text error :reason error))
         (list (list :type 'handshake :state (list :turn (gethash "id" (gethash "turn" result)))))))
      ("turn/steer" (romacs-agent--codex-steered session id error))
      ;; An interrupt that found the turn over has nothing to say.
      ("turn/interrupt" nil)
      (_ (and error (list (list :type 'error :text error)))))))

(defun romacs-agent--codex-steered (session id error)
  "Steer ID answered: done with, or, failed with ERROR (its turn ended
first), owed as a turn of its own."
  (let* ((state (romacs-agent-driver-state session))
         (steer (assoc id (plist-get state :steers)))
         (steers (remove steer (plist-get state :steers))))
    (if (not (and error steer))
        (list (list :type 'handshake :state (list :steers steers)))
      (list (list :type 'status :text (format "Not steered (%s): sent as a new turn" error))
            (list :type 'handshake
                  :state (list :steers steers
                               :owed (append (plist-get state :owed) (list (cons :turn steer)))))))))

(defun romacs-agent--codex-asked (session id method params)
  "The events of request ID, METHOD with PARAMS, Codex asks of Romacs:
an approval for the user, anything else refused."
  (cl-flet ((approval (tool input)
              (puthash id (cons method params) (romacs-agent-approvals session))
              (list (list :type 'approval :request-id id :tool tool :input input
                          :description (romacs-agent--value (gethash "reason" params))))))
    (pcase method
      ("item/commandExecution/requestApproval"
       (approval "Bash" (romacs-mcp--object "command" (romacs-agent--value (gethash "command" params))
                                            "cwd" (romacs-agent--value (gethash "cwd" params)))))
      ("item/fileChange/requestApproval"
       (approval "Edit" (romacs-mcp--object "grantRoot" (or (romacs-agent--value (gethash "grantRoot" params)) :null))))
      ("item/permissions/requestApproval" (approval "Permissions" (gethash "permissions" params)))
      (_ (list (list :type 'handshake
                     :state (list :owed (append (plist-get (romacs-agent-driver-state session) :owed)
                                                (list (list :reply id method))))))))))

(defun romacs-agent--codex-tool (item)
  "An item's tool name and input, for the tool block."
  (pcase (gethash "type" item)
    ("commandExecution" (list "Bash" (romacs-mcp--object "command" (gethash "command" item))))
    ("mcpToolCall" (list (format "mcp__%s__%s" (gethash "server" item) (gethash "tool" item))
                         (gethash "arguments" item)))
    ("dynamicToolCall" (list (gethash "tool" item) (gethash "arguments" item)))
    ("fileChange" (list "Edit" (romacs-mcp--object "changes" (gethash "changes" item))))
    ("webSearch" (list "WebSearch" (romacs-mcp--object "query" (gethash "query" item))))
    (type (list type nil))))

(defun romacs-agent--codex-output (item)
  (let ((output (or (romacs-agent--value (gethash "aggregatedOutput" item))
                    (romacs-agent--value (gethash "result" item))
                    (romacs-agent--value (gethash "contentItems" item))
                    (romacs-agent--value (gethash "error" item)) "")))
    (cond ((stringp output) output)
          ((vectorp output) (romacs-agent--text-of output))
          ((and (hash-table-p output) (gethash "content" output)) (romacs-agent--text-of (gethash "content" output)))
          ((and (hash-table-p output) (gethash "message" output)) (gethash "message" output))
          (t (romacs-agent--json output)))))

(defun romacs-agent--codex-tool-events (session id item completed)
  "The tool block of ITEM, by ID: started once, and its end when COMPLETED."
  (let ((started (unless (gethash id (romacs-agent-tools session))
                   (puthash id t (romacs-agent-tools session))
                   (pcase-let ((`(,name ,input) (romacs-agent--codex-tool item)))
                     (list (list :type 'tool :state 'started :id id :name name :input input)))))
        (failed (or (member (gethash "status" item) '("failed" "declined"))
                    (eq (gethash "success" item) :false)
                    (let ((exit (gethash "exitCode" item))) (and (integerp exit) (/= 0 exit))))))
    (append started
            (when completed
              (list (list :type 'tool :state (if failed 'failed 'completed) :id id
                          :output (romacs-agent--codex-output item)))))))

(defun romacs-agent--codex-item (session item completed &optional id)
  "The events of ITEM, started or COMPLETED; ID, when given, names it."
  (let ((id (or id (gethash "id" item)))
        (streamed (romacs-agent-streamed session)))
    (pcase (gethash "type" item)
      ("agentMessage"
       (when completed
         (setf (romacs-agent-reply session) (gethash "text" item))
         (list (list :type 'delta :text (if (gethash id streamed) "\n" (concat (gethash "text" item) "\n"))
                     :message id)
               (list :type 'message-end :message id))))
      ("reasoning"
       (when (and completed (not (gethash id streamed)))
         (let ((text (mapconcat #'identity (append (or (romacs-agent--value (gethash "summary" item))
                                                       (romacs-agent--value (gethash "content" item))
                                                       (and (stringp (gethash "text" item)) (list (gethash "text" item))))
                                                   nil)
                                "\n\n")))
           (unless (string-empty-p text) (list (list :type 'thinking :text text))))))
      ("contextCompaction" (when completed (list (list :type 'status :text "Context compacted"))))
      ((or "userMessage" "hookPrompt" "plan" "todo_list" "error") nil)
      (_ (romacs-agent--codex-tool-events session id item completed)))))

(defun romacs-agent--codex-plan (params)
  "A plan update, as a status line: each step marked by its state."
  (let ((steps (mapconcat (lambda (step)
                            (concat (pcase (gethash "status" step) ("completed" "✓ ") ("inProgress" "→ ") (_ "○ "))
                                    (gethash "step" step)))
                          (gethash "plan" params) "  ")))
    (concat "Plan: " (if-let* ((why (romacs-agent--value (gethash "explanation" params)))) (concat why "  ") "") steps)))

(defun romacs-agent--codex-done (session turn)
  "The end of TURN, as `turn/completed' gives it."
  (let* ((status (gethash "status" turn))
         (reason (or (romacs-agent--codex-error turn) (and (equal status "interrupted") "Interrupted")))
         (ms (gethash "durationMs" turn)))
    (list :type 'done :text (romacs-agent-reply session) :error (not (equal status "completed"))
          :reason reason :duration (and (numberp ms) (/ ms 1000.0))
          :context-window (plist-get (romacs-agent-driver-state session) :window))))

(defun romacs-agent--codex-notified (session method params)
  "The events of notification METHOD with PARAMS."
  (let ((state (romacs-agent-driver-state session)))
    (pcase method
      ("turn/started"
       (setf (romacs-agent-reply session) nil)
       (list (list :type 'handshake :state (list :turn (gethash "id" (gethash "turn" params))))))
      ("turn/completed"
       (let ((turn (gethash "turn" params)))
         (append (when (equal (gethash "id" turn) (plist-get state :turn))
                   (list (list :type 'handshake :state (list :turn nil))))
                 (list (romacs-agent--codex-done session turn)))))
      ("item/started" (romacs-agent--codex-item session (gethash "item" params) nil))
      ("item/completed" (romacs-agent--codex-item session (gethash "item" params) t))
      ("item/agentMessage/delta"
       (puthash (gethash "itemId" params) t (romacs-agent-streamed session))
       (list (list :type 'delta :text (gethash "delta" params) :message (gethash "itemId" params))))
      ("item/reasoning/summaryTextDelta"
       (puthash (gethash "itemId" params) t (romacs-agent-streamed session))
       (list (list :type 'thinking :text (gethash "delta" params))))
      ("item/reasoning/summaryPartAdded"
       (when (> (gethash "summaryIndex" params 0) 0) (list (list :type 'thinking :text "\n\n"))))
      ("turn/plan/updated" (list (list :type 'status :text (romacs-agent--codex-plan params))))
      ("thread/tokenUsage/updated"
       (let* ((usage (gethash "tokenUsage" params))
              (window (romacs-agent--value (gethash "modelContextWindow" usage))))
         (append (list (list :type 'usage :context-used (gethash "totalTokens" (gethash "last" usage))))
                 (when (and window (not (eql window (plist-get state :window))))
                   (list (list :type 'handshake :state (list :window window)))))))
      ("error"
       (let ((text (romacs-agent--codex-error params)))
         (if (eq (gethash "willRetry" params) t)
             (list (list :type 'status :text (format "Retrying: %s" text)))
           (list (list :type 'error :text text)))))
      ("serverRequest/resolved"
       ;; Settled without the user (its turn ended): no longer theirs to answer.
       (let ((id (gethash "requestId" params)))
         (when (gethash id (romacs-agent-approvals session))
           (remhash id (romacs-agent-approvals session))
           (list (list :type 'answered :request-id id)))))
      ("model/rerouted"
       (list (list :type 'status :text (format "Model rerouted to %s" (gethash "toModel" params)))))
      ((or "warning" "configWarning" "deprecationNotice")
       (list (list :type 'status :text (or (gethash "message" params) (gethash "summary" params)))))
      ((or "thread/started" "thread/status/changed" "thread/settings/updated" "thread/compacted"
           "mcpServer/startupStatus/updated" "remoteControl/status/changed" "skills/changed"
           "item/commandExecution/outputDelta" "item/fileChange/outputDelta" "item/fileChange/patchUpdated"
           "item/reasoning/textDelta" "item/plan/delta" "item/mcpToolCall/progress"
           "hook/started" "hook/completed" "turn/diff/updated")
       nil)
      ((pred (string-prefix-p "account/")) nil)
      (_ (list (list :type 'unknown :raw params))))))

;;; `codex exec --json''s lines, as earlier sessions journaled them

(defun romacs-agent--codex-exec-item (item)
  "An exec run's ITEM in the app-server's words."
  (let ((item (copy-hash-table item)))
    (pcase-dolist (`(,old . ,new) '(("aggregated_output" . "aggregatedOutput") ("exit_code" . "exitCode")))
      (when-let* ((value (gethash old item))) (puthash new value item)))
    (puthash "type" (pcase (gethash "type" item)
                      ("agent_message" "agentMessage") ("command_execution" "commandExecution")
                      ("mcp_tool_call" "mcpToolCall") ("file_change" "fileChange") ("web_search" "webSearch")
                      (type type))
             item)
    item))

(defun romacs-agent--codex-exec-events (session line)
  "The events of LINE, one of `codex exec --json''s objects."
  (cl-flet ((item (completed)
              ;; Its ids count from item_0 every run.
              (let ((item (gethash "item" line)))
                (romacs-agent--codex-item session (romacs-agent--codex-exec-item item) completed
                                          (format "%d:%s" (romacs-agent-turns session) (gethash "id" item))))))
    (pcase (gethash "type" line)
      ("thread.started" (list (list :type 'session :harness-session (gethash "thread_id" line)
                                    :model (plist-get (romacs-agent-options session) :model))))
      ("turn.started" (setf (romacs-agent-reply session) nil) nil)
      ("item.started" (item nil))
      ("item.updated" nil)
      ("item.completed" (item t))
      ("turn.completed" (list (list :type 'done :text (romacs-agent-reply session))))
      ("turn.failed" (list (list :type 'done :error t :text (or (romacs-agent--codex-error line) "turn failed"))))
      ("error" (list (list :type 'error :text (gethash "message" line))))
      (_ (list (list :type 'unknown :raw line))))))

(defun romacs-agent--codex-events (session line)
  "The events of LINE, a JSON-RPC message from `codex app-server' (or a
line an earlier `codex exec' run journaled)."
  (let ((method (gethash "method" line)) (id (gethash "id" line)))
    (cond ((gethash "type" line) (romacs-agent--codex-exec-events session line))
          ((and method id) (romacs-agent--codex-asked session id method (gethash "params" line)))
          (method (romacs-agent--codex-notified session method (gethash "params" line)))
          (t (romacs-agent--codex-answer session id line)))))

(romacs-agent-define-harness 'codex
  :display "Codex" :icon "◎" :t3-driver "codex"
  :config-variable "CODEX_HOME" :config-default "~/.codex"
  :program-option 'romacs-agent-codex-program :arguments-option 'romacs-agent-codex-arguments
  ;; What Codex sets for the commands it runs (codex-cli 0.164, read from
  ;; a child's environment).
  :scrub '("CODEX_APPLY_PATCH_PRESERVE_LINE_ENDINGS" "CODEX_CI" "CODEX_SANDBOX_NETWORK_DISABLED"
           "CODEX_SESSION_ID" "CODEX_THREAD_ID" "CODEX_TOOL_CALL_ID" "CODEX_VERSION")
  :lifetime 'persistent
  :select #'romacs-agent--codex-select
  :argv #'romacs-agent--codex-argv
  :next #'romacs-agent--codex-next
  :input #'romacs-agent--codex-input
  :interrupt #'romacs-agent--codex-interrupt
  :respond #'romacs-agent--codex-respond
  :events #'romacs-agent--codex-events
  :input-events #'romacs-agent--codex-input-events
  :native-id-p #'romacs-agent--uuid-p
  :history '(:files romacs-agent-history--codex-files :keep romacs-agent-history--codex-keep
             :describe romacs-agent-history--codex-describe :events romacs-agent-history--codex-events))

(provide 'romacs-agent-codex)
;;; romacs-agent-codex.el ends here
