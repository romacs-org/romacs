;;; romacs-agent-claude.el --- Claude Code, as Romacs runs it  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Claude Code's driver (`romacs-agent-define-harness'): one `claude -p'
;; process per session, stream-json lines both ways.  Its approvals come
;; to Romacs (--permission-prompt-tool stdio) and are answered by
;; control_response; an interrupt is a control_request.

;;; Code:

(require 'romacs-agent)
(require 'romacs-agent-gate)

(defun romacs-agent--claude-forking (session)
  "Whether SESSION, a fork, still has to be made: Claude Code writes the
fork's transcript with its first message, so until one was sent every
run forks its source again."
  (and (plist-get (romacs-agent-options session) :fork) (not (romacs-agent-input-seq session))))

(defun romacs-agent--claude-argv (session resume)
  "Claude Code's command line for SESSION, resuming its conversation when
RESUME is non-nil."
  (let ((dir (romacs-agent-dir session))
        (instance (romacs-agent--instance (romacs-agent-provider session) (romacs-agent-harness session))))
    (romacs-agent--write-json
     (expand-file-name "mcp.json" dir)
     (romacs-mcp--object
      "mcpServers" (romacs-mcp--object
                    "romacs" (romacs-mcp--object
                              "command" (romacs-agent--program "romacs-mcp")
                              "args" (vector "--socket" (romacs-agent--route session) "--reconnect"
                                             "--session" (romacs-agent-id session)
                                             "--reply-timeout" (number-to-string romacs-agent-reply-timeout))
                              ;; Claude Code's own limit on a call, in ms: past
                              ;; the relay's, so the relay's answer is the one.
                              "timeout" (* 1000 (+ (truncate romacs-agent-reply-timeout) 15))))))
    (append (romacs-agent--environment-words session)
            (list (format "MCP_TOOL_TIMEOUT=%d" (* 1000 (+ (truncate romacs-agent-reply-timeout) 15))))
            (list (plist-get instance :program) "-p" "--output-format" "stream-json"
                  "--input-format" "stream-json" "--verbose" "--include-partial-messages"
                  ;; Without it the stream carries thinking blocks with no text.
                  "--thinking-display" "summarized"
                  ;; The harness's own approvals come to this session's UI.
                  "--permission-prompt-tool" "stdio"
                  "--mcp-config" (expand-file-name "mcp.json" dir)
                  "--append-system-prompt" (romacs-agent--instructions session))
            (cond ((romacs-agent--claude-forking session)
                   (list "--resume" (plist-get (romacs-agent-options session) :fork) "--fork-session"
                         "--session-id" (romacs-agent-id session)))
                  (resume (list "--resume" (romacs-agent--native-id session)))
                  (t (list "--session-id" (romacs-agent-id session))))
            (romacs-agent--option-words session "--model" "--effort")
            (plist-get instance :arguments))))

(defun romacs-agent--claude-image (file)
  "FILE as a message's image content block."
  (romacs-mcp--object
   "type" "image"
   "source" (romacs-mcp--object
             "type" "base64"
             "media_type" (pcase (downcase (file-name-extension file))
                            ((or "jpg" "jpeg") "image/jpeg") ("gif" "image/gif") ("webp" "image/webp") (_ "image/png"))
             "data" (with-temp-buffer
                      (set-buffer-multibyte nil)
                      (insert-file-contents-literally file)
                      (base64-encode-region (point-min) (point-max) t)
                      (buffer-string)))))

(defun romacs-agent--claude-user (_session text _id)
  "The harness's message of TEXT, with the images attached to it as content,
each after a line naming where it is saved: the model can refer to it."
  (let ((images (seq-filter #'romacs-agent-image-file-p romacs-agent--message-images)))
    (romacs-mcp--object
     "type" "user"
     "message" (romacs-mcp--object
                "role" "user"
                "content" (if images
                              (vconcat (list (romacs-mcp--object "type" "text" "text" text))
                                       (mapcan (lambda (file)
                                                 (list (romacs-mcp--object
                                                        "type" "text"
                                                        "text" (romacs-agent--image-caption file))
                                                       (romacs-agent--claude-image file)))
                                               images))
                            text)))))

(defun romacs-agent--claude-reason (result)
  "Why RESULT, a turn's end, failed, in the harness's words, or nil."
  (when (eq (gethash "is_error" result) t)
    (let ((errors (append (romacs-agent--value (gethash "errors" result)) nil)))
      (if errors
          (mapconcat (lambda (e) (if (stringp e) e (romacs-agent--json e))) errors "; ")
        (romacs-agent--value (gethash "result" result))))))

(defun romacs-agent--claude-status (line)
  "The events of LINE, a status: a compaction's end has no status of its own."
  (let ((status (romacs-agent--value (gethash "status" line))))
    (cond ((equal (gethash "compact_result" line) "failed")
           (list (list :type 'error :text (format "Compaction failed: %s" (gethash "compact_error" line)))))
          (status (list (list :type 'status :text status))))))

(defun romacs-agent--claude-injected (text parent)
  "The events of TEXT, a local command's output nobody typed.  The compaction's
own \"Compacted\" is already said by its row."
  (let ((output (string-trim (replace-regexp-in-string "</?local-command-\\(?:stdout\\|stderr\\)>" "" text))))
    (unless (member output '("" "Compacted"))
      (list (list :type 'status :text output :parent parent)))))

(defun romacs-agent--claude-task (line)
  "LINE, a notice of work the harness runs under a tool call (a subagent,
or a command), as a `task' event: its tool's :id, then what it says:
started (in the background or not), what it is doing, moved to the
background (known by its :task alone), or how it ended.  Work a subagent
runs is that subagent's: no event."
  (let ((id (romacs-agent--value (gethash "tool_use_id" line)))
        (patch (gethash "patch" line)))
    (cond
     ((or (romacs-agent--value (gethash "parent_task_id" line))
          (eq (gethash "owned_by_subagent" line) t))
      nil)
     ((equal (gethash "subtype" line) "task_updated")
      (when (and (hash-table-p patch) (eq (gethash "is_backgrounded" patch) t))
        (list (list :type 'task :task (gethash "task_id" line) :state 'backgrounded))))
     (id
      (list (append (list :type 'task :id id)
                  (pcase (gethash "subtype" line)
                    ("task_started" (list :state 'started :task (gethash "task_id" line)
                                          :title (romacs-agent--value (gethash "description" line))
                                          :background (or (eq (gethash "is_backgrounded" line) t)
                                                          (equal (gethash "task_type" line) "local_agent"))))
                    ("task_progress" (list :state 'progress
                                           :activity (or (romacs-agent--value (gethash "last_tool_name" line))
                                                         (romacs-agent--value (gethash "description" line)))))
                    (_ (list :state (pcase (gethash "status" line)
                                      ("completed" 'completed) ("failed" 'failed) (_ 'stopped))
                             :summary (romacs-agent--value (gethash "summary" line)))))))))))

(defun romacs-agent--claude-mode-refused (session id response)
  "The events of Claude Code refusing mode switch ID, RESPONSE: said; a
switch into plan mode leaves the session out of it, and one out of plan
mode is tried again to the default."
  (let ((asked (plist-get (romacs-agent-driver-state session) :mode-asked)))
    (cons (list :type 'error :text (format "Claude Code would not switch to %s mode: %s"
                                           (or asked "that") (or (romacs-agent--value (gethash "error" response)) "")))
          (if (equal asked "plan") (list (list :type 'plan-refused))
            (unless (member asked '(nil "default"))
            (list (list :type 'handshake
                        :state (list :owed (append (plist-get (romacs-agent-driver-state session) :owed)
                                                   (list (list :line (format "romacs-mode-default-%s" id) "default")))))))))))

(defun romacs-agent--claude-plan (session on from)
  "The line putting SESSION's Claude Code in plan mode when ON, else back
in mode FROM (the default when it had none)."
  (romacs-mcp--object "type" "control_request"
                      "request_id" (format "romacs-mode-%d" (romacs-agent-request-id session))
                      "request" (romacs-mcp--object "subtype" "set_permission_mode"
                                                    "mode" (if on "plan" (or from "default")))))

(defun romacs-agent--claude-cache-ttl (usage)
  "Seconds the prompt cache a request with USAGE wrote lives: an hour or
five minutes, as it says; nil when it wrote none (served wholly from the
cache, it reports none) or does not say."
  (let ((made (and (hash-table-p usage) (gethash "cache_creation" usage))))
    (when (hash-table-p made)
      (cl-flet ((wrote (key) (let ((n (gethash key made))) (and (numberp n) (> n 0)))))
        (cond ((wrote "ephemeral_1h_input_tokens") 3600)
              ((wrote "ephemeral_5m_input_tokens") 300))))))

(cl-defun romacs-agent--claude-events (session line)
  "The events of LINE, one of Claude Code's stream-json objects."
  (let ((type (gethash "type" line))
        (parent (romacs-agent--value (gethash "parent_tool_use_id" line)))
        (compacting (romacs-agent-compacting session)))
    ;; Only the next line can be the summary: a compaction cut short leaves
    ;; no mark for a later run's line to match.
    (setf (romacs-agent-compacting session) nil)
    (pcase type
      ("system"
       (pcase (gethash "subtype" line)
         ("init" (list (list :type 'session :model (gethash "model" line)
                             :harness-session (gethash "session_id" line)
                             :permission-mode (gethash "permissionMode" line)
                             :mcp (gethash "mcp_servers" line))))
         ("status" (append (and (stringp (gethash "permissionMode" line))
                                (list (list :type 'mode :permission-mode (gethash "permissionMode" line))))
                           (romacs-agent--claude-status line)))
         ("compact_boundary" (setf (romacs-agent-compacting session) t) nil)
         ((or "task_started" "task_progress" "task_notification" "task_updated") (romacs-agent--claude-task line))
         (_ nil)))
      ("stream_event"
       (let* ((event (gethash "event" line))
              (kind (and event (gethash "type" event))))
         (pcase kind
           ("message_start"
            (setf (romacs-agent-message session) (gethash "id" (gethash "message" event)))
            ;; A top-level request: how long the prompt cache it wrote lives.
            (unless parent
              (list (list :type 'cache :ttl (romacs-agent--claude-cache-ttl (gethash "usage" (gethash "message" event)))))))
           ("content_block_delta"
            (let* ((delta (gethash "delta" event)) (dtype (gethash "type" delta)))
              (puthash (romacs-agent-message session) t (romacs-agent-streamed session))
              (pcase dtype
                ("text_delta" (list (list :type 'delta :text (gethash "text" delta) :parent parent
                                          :message (romacs-agent-message session))))
                ("thinking_delta" (list (list :type 'thinking :text (gethash "thinking" delta) :parent parent))))))
           (_ nil))))
      ("assistant"
       (let* ((message (gethash "message" line))
              (id (gethash "id" message))
              (streamed (gethash id (romacs-agent-streamed session)))
              (usage (gethash "usage" message))
              (text nil))
         (append
          ;; The prompt this call sent: the context it fills.
          (when (and (hash-table-p usage) (not parent))
            (list (list :type 'usage :context-used (+ (or (gethash "input_tokens" usage) 0)
                                                      (or (gethash "cache_read_input_tokens" usage) 0)
                                                      (or (gethash "cache_creation_input_tokens" usage) 0)))))
          (cl-loop for block across (gethash "content" message)
                   for btype = (gethash "type" block)
                   when (equal btype "text") do (setq text t)
                   when (and (equal btype "thinking") (not streamed))
                   collect (list :type 'thinking :text (gethash "thinking" block) :parent parent)
                   when (and (equal btype "text") (not streamed))
                   collect (list :type 'delta :text (gethash "text" block) :parent parent :message id)
                   when (equal btype "tool_use")
                   collect (progn
                             (puthash (gethash "id" block) (or parent t) (romacs-agent-tools session))
                             (list :type 'tool :state 'started :id (gethash "id" block)
                                   :name (gethash "name" block) :input (gethash "input" block)
                                   :parent parent)))
          ;; The message's text is whole: its view may render it once.
          (when text (list (list :type 'message-end :message id :parent parent))))))
      ("user"
       (let ((content (gethash "content" (gethash "message" line))))
         (cond
          ;; The synthetic line right after a boundary is its summary: a
          ;; string from /compact, text blocks from an automatic compaction.
          ((and compacting (eq (gethash "isSynthetic" line) t))
           (list (list :type 'compacted :text (romacs-agent--text-of content))))
          ((stringp content) (romacs-agent--claude-injected content parent))
          (t
           (cl-loop for block across content
                    for btype = (gethash "type" block)
                    when (equal btype "tool_result")
                    collect (list :type 'tool :state (if (eq (gethash "is_error" block) t) 'failed 'completed)
                                  :id (gethash "tool_use_id" block)
                                  :output (romacs-agent--text-of (gethash "content" block))
                                  :parent parent)
                    when (equal btype "text")
                    collect (list :type 'status :text (gethash "text" block) :parent parent))))))
      ("result"
       (list (list :type 'done :cost (gethash "total_cost_usd" line)
                   :error (eq (gethash "is_error" line) t) :text (gethash "result" line)
                   :reason (romacs-agent--claude-reason line)
                   :duration (let ((ms (gethash "duration_ms" line))) (and (numberp ms) (/ ms 1000.0)))
                   :context-window (let ((usage (gethash "modelUsage" line)) (window nil))
                                     (when (hash-table-p usage)
                                       (maphash (lambda (_ model)
                                                  (let ((size (and (hash-table-p model) (gethash "contextWindow" model))))
                                                    (when (numberp size) (setq window (max size (or window 0))))))
                                                usage))
                                     window))))
      ("control_request"
       (let ((request (gethash "request" line)))
         (when (equal (gethash "subtype" request) "hook_callback")
           ;; Answered when written, live: a replay judges nothing.
           (cl-return-from romacs-agent--claude-events
             (list (list :type 'handshake
                         :state (list :owed (append (plist-get (romacs-agent-driver-state session) :owed)
                                                    (list (list :hook (gethash "request_id" line) request))))))))
         (when (equal (gethash "subtype" request) "can_use_tool")
           (puthash (gethash "request_id" line) request (romacs-agent-approvals session))
           (append
            ;; A plan presented: shown, and its call stopped (`romacs-agent--plan-captured').
            (let ((plan (and (equal (gethash "tool_name" request) "ExitPlanMode") (hash-table-p (gethash "input" request))
                             (romacs-agent--value (gethash "plan" (gethash "input" request))))))
              (and (stringp plan) (not (string-empty-p (string-trim plan))) (list (list :type 'plan :text plan))))
            (list (list :type 'approval :request-id (gethash "request_id" line)
                       :plan (equal (gethash "tool_name" request) "ExitPlanMode")
                       :tool (gethash "tool_name" request) :input (gethash "input" request)
                       ;; A question, or a plan to approve, is the user's alone.
                       :user (and (member (gethash "tool_name" request) '("AskUserQuestion" "ExitPlanMode")) t)
                       :questions (and (equal (gethash "tool_name" request) "AskUserQuestion")
                                       (romacs-agent--claude-questions (gethash "input" request)))
                       :description (gethash "description" request)))))))
      ("control_response"
       (let* ((response (gethash "response" line)) (id (gethash "request_id" response)))
         (cond ((and id (romacs-agent--select-answered session id (equal (gethash "subtype" response) "success"))))
               ;; This run's initialize answered, however: its hooks are on
               ;; when it took them, and messages may go.
               ((and id (equal id (plist-get (romacs-agent-driver-state session) :initialize)))
                (when (hash-table-p (gethash "response" response))
                  (romacs-agent--keep-models session (romacs-agent--claude-models (gethash "response" response))))
                (cl-return-from romacs-agent--claude-events
                  (list (list :type 'handshake :state (list :initialized t)))))
               ((and id (gethash id (romacs-agent-approvals session)))
                (remhash id (romacs-agent-approvals session)))
               ;; A mode it would not take: out of plan mode, back to the
               ;; default rather than stay (T3 Code's fallback).
               ((and id (string-prefix-p "romacs-mode-" id) (equal (gethash "subtype" response) "error"))
                (cl-return-from romacs-agent--claude-events (romacs-agent--claude-mode-refused session id response))))
         nil))
      ("rate_limit_event" nil)
      (_ (list (list :type 'unknown :raw line))))))

(defun romacs-agent--claude-models (answer)
  "The model rows in ANSWER, what Claude Code says to `initialize': each
model by the id its alias resolves to, its label and efforts and nothing
else of the answer, which also describes the account.  The \"default\"
alias is no model."
  (let ((models (gethash "models" answer)))
    (cl-loop for model across (if (vectorp models) models [])
             for id = (or (romacs-agent--value (gethash "resolvedModel" model)) (gethash "value" model))
             for efforts = (romacs-agent--value (gethash "supportedEffortLevels" model))
             when (and (stringp id) (not (equal (gethash "value" model) "default")))
             ;; No efforts is what it says too: not left to the catalogue.
             collect (cons id (list :label (or (romacs-agent--value (gethash "displayName" model)) id)
                                    :efforts (and (vectorp efforts) (append efforts nil)))))))

(defun romacs-agent--claude-select (session model effort)
  "The control requests moving SESSION's idle Claude Code to MODEL at
EFFORT.  Neither can be cleared in a live harness: `restart' then."
  (let ((now (romacs-agent--selection session)))
    (if (or (and (plist-get now :model) (not model)) (and (plist-get now :effort) (not effort)))
        'restart
      (cl-flet ((request (subtype &rest fields)
                  (let ((id (format "romacs-select-%d" (romacs-agent-request-id session)))
                        (body (romacs-mcp--object "subtype" subtype)))
                    (cl-loop for (name value) on fields by #'cddr do (puthash name value body))
                    (cons id (romacs-mcp--object "type" "control_request" "request_id" id "request" body)))))
        (append (unless (equal model (plist-get now :model))
                  (list (request "set_model" "model" model)))
                (unless (equal effort (plist-get now :effort))
                  (list (request "apply_flag_settings" "settings" (romacs-mcp--object "effortLevel" effort)))))))))

;; A record's image files are trusted as what the user attached: the journal
;; is this user's private state, whoever can rewrite it can read those files
;; anyway, and a recalled image shows in the tray before anything is sent.
(defun romacs-agent--claude-input-events (session line)
  "The events of LINE, written to SESSION's harness: a user's prompt, or
an approval answered."
  (pcase (gethash "type" line)
    ("user" (list (list :type 'input :text (romacs-agent--text-of (gethash "content" (gethash "message" line)))
                        :images (append (gethash "images" line) nil))
                  ;; Spoken to before any initialize: a run of before, ready as it is.
                  (list :type 'handshake :state (list :spoken t))))
    ("control_request"
     (let ((owed (plist-get (romacs-agent-driver-state session) :owed)))
       (pcase (gethash "subtype" (gethash "request" line))
         ("initialize" (list (list :type 'handshake :state (list :initialize (gethash "request_id" line)))))
         ("set_permission_mode"
          (list (list :type 'handshake
                      :state (list :mode-asked (gethash "mode" (gethash "request" line))
                                   :owed (cl-remove (gethash "request_id" line) owed :key #'cadr :test #'equal))))))))
    ("control_response"
     (let* ((id (gethash "request_id" (gethash "response" line)))
            (owed (plist-get (romacs-agent-driver-state session) :owed))
            (why (let ((out (gethash "response" (gethash "response" line))))
                   (and (hash-table-p out) (hash-table-p (gethash "hookSpecificOutput" out))
                        (gethash "permissionDecisionReason" (gethash "hookSpecificOutput" out))))))
       (if (cl-find id owed :key #'cadr :test #'equal)
           ;; A reply it owed, written: owed no more; a refusal is shown.
           (append (and (stringp why) (list (list :type 'status :text why)))
                   (list (list :type 'handshake :state (list :owed (cl-remove id owed :key #'cadr :test #'equal)))))
         (remhash id (romacs-agent-approvals session))
         (list (list :type 'answered :request-id id
                     :behavior (gethash "behavior" (gethash "response" (gethash "response" line))))))))
    (_ nil)))

;;; Romacs on Claude Code's own hooks

(defconst romacs-agent--claude-hooks
  (romacs-mcp--object "PreToolUse" (vector (romacs-mcp--object "matcher" "Edit|Write|MultiEdit"
                                                               "hookCallbackIds" ["romacs-gate"])))
  "The hooks Romacs takes on each run: Claude Code asks it about every
edit before writing it (`romacs-agent-gate-check').")

(defun romacs-agent--claude-next (session)
  "SESSION's next driver line: a reply it owes Claude Code, else this
run's initialize, with Romacs's hooks; ready once that is answered."
  (let ((state (romacs-agent-driver-state session)))
    (cond ((plist-get state :owed) (romacs-agent--claude-reply session (car (plist-get state :owed))))
          ;; In plan mode, the harness not: put it there, once, whatever
          ;; started it (a launch, a resume, a detached session attached).
          ((and (plist-get state :initialized) (plist-get (romacs-agent-options session) :plan)
                (not (equal (romacs-agent-permission-mode session) "plan"))
                (not (equal (plist-get state :mode-asked) "plan")))
           (romacs-agent--claude-plan session t nil))
          ((or (plist-get state :initialized) (plist-get state :spoken)) :ready)
          ((plist-get state :initialize) nil)
          (t (romacs-mcp--object
              "type" "control_request"
              "request_id" (format "romacs-init-%d" (romacs-agent-request-id session))
              "request" (romacs-mcp--object "subtype" "initialize" "hooks" romacs-agent--claude-hooks))))))

(defun romacs-agent--claude-hook (request)
  "The answer to hook REQUEST: the gate's refusal of an edit, else nothing
to say.  Judged as it is written, on the file as it is then."
  (let* ((input (gethash "input" request))
         (why (and (hash-table-p input) (equal (gethash "hook_event_name" input) "PreToolUse")
                   (romacs-agent-gate-check (gethash "tool_name" input) (gethash "tool_input" input)))))
    (if why
        (romacs-mcp--object "hookSpecificOutput"
                            (romacs-mcp--object "hookEventName" "PreToolUse" "permissionDecision" "deny"
                                                "permissionDecisionReason" why))
      (romacs-mcp--object))))

(defun romacs-agent--claude-reply (_session owed)
  "The line OWED asks for, written live only: (:hook ID REQUEST), the
answer to a hook; (:line ID MODE), a switch to permission mode MODE."
  (pcase (car owed)
    (:line (romacs-mcp--object "type" "control_request" "request_id" (cadr owed)
                               "request" (romacs-mcp--object "subtype" "set_permission_mode" "mode" (nth 2 owed))))
    (_ (romacs-mcp--object "type" "control_response"
                           "response" (romacs-mcp--object "subtype" "success" "request_id" (cadr owed)
                                                          "response" (romacs-agent--claude-hook (nth 2 owed)))))))

(defun romacs-agent--claude-interrupt (session)
  (romacs-mcp--object "type" "control_request"
                      "request_id" (format "romacs-interrupt-%d" (romacs-agent-request-id session))
                      "request" (romacs-mcp--object "subtype" "interrupt")))

(defun romacs-agent--claude-questions (input)
  "AskUserQuestion's INPUT as an `approval' event's :questions: each
\(QUESTION :header HEADER :options LABELS :notes ((LABEL . DESCRIPTION) ...)
:multi MULTI)."
  (let ((questions (and (hash-table-p input) (gethash "questions" input))))
    (cl-loop for q across (if (vectorp questions) questions [])
             for options = (cl-loop for o across (let ((o (gethash "options" q))) (if (vectorp o) o []))
                                    collect (cons (gethash "label" o) (romacs-agent--value (gethash "description" o))))
             when (stringp (gethash "question" q))
             collect (list (gethash "question" q)
                           :header (romacs-agent--value (gethash "header" q))
                           :options (mapcar #'car options)
                           :notes (cl-remove-if-not #'cdr options)
                           :multi (eq (gethash "multiSelect" q) t)))))

(defun romacs-agent--claude-session-rules (request)
  "The permission updates allowing REQUEST's tool for the rest of the
session: what Claude Code suggests, kept to the session, else its tool."
  (let ((suggestions (gethash "permission_suggestions" request)))
    (if (and (vectorp suggestions) (> (length suggestions) 0))
        (vconcat (mapcar (lambda (suggestion)
                           (let ((update (copy-hash-table suggestion)))
                             (puthash "destination" "session" update)
                             update))
                         suggestions))
      (vector (romacs-mcp--object "type" "addRules" "behavior" "allow" "destination" "session"
                                  "rules" (vector (romacs-mcp--object "toolName" (gethash "tool_name" request))))))))

(defun romacs-agent--claude-respond (session request-id allow message)
  "The control_response answering approval REQUEST-ID: denied with MESSAGE
when ALLOW is nil, else allowed (`romacs-agent-respond'): with the user's
answers in its input, and for the session when asked."
  (let* ((request (gethash request-id (romacs-agent-approvals session)))
         (input (copy-hash-table (or (gethash "input" request) (romacs-mcp--object))))
         (answers (and (consp allow) (plist-get allow :answers))))
    (when answers
      (puthash "answers" (let ((object (romacs-mcp--object)))
                           (dolist (answer answers object) (puthash (car answer) (cdr answer) object)))
               input))
    (romacs-mcp--object
     "type" "control_response"
     "response" (romacs-mcp--object
                 "subtype" "success" "request_id" request-id
                 "response" (cond ((null allow)
                                   (romacs-mcp--object "behavior" "deny" "message" (or message "Denied by the user")))
                                  ((and (consp allow) (plist-get allow :always))
                                   (romacs-mcp--object "behavior" "allow" "updatedInput" input
                                                       "updatedPermissions" (romacs-agent--claude-session-rules request)))
                                  (t (romacs-mcp--object "behavior" "allow" "updatedInput" input)))))))

(romacs-agent-define-harness 'claude
  :display "Claude Code" :icon "✳" :t3-driver "claudeAgent"
  :config-variable "CLAUDE_CONFIG_DIR" :config-default "~/.claude"
  :program-option 'romacs-agent-claude-program :arguments-option 'romacs-agent-claude-arguments
  ;; What Claude Code sets for the commands it runs (a parent session's).
  :scrub '("CLAUDECODE" "CLAUDE_AGENT_SDK_VERSION" "CLAUDE_CODE_CHILD_SESSION" "CLAUDE_CODE_ENTRYPOINT"
           "CLAUDE_CODE_EXECPATH" "CLAUDE_CODE_MESSAGING_SOCKET" "CLAUDE_CODE_MESSAGING_TOKEN"
           "CLAUDE_CODE_SESSION_ATTENDED" "CLAUDE_CODE_SESSION_ID" "CLAUDE_EFFORT" "CLAUDE_PID")
  :lifetime 'persistent :forks #'romacs-agent--claude-forking
  :next #'romacs-agent--claude-next
  :plan #'romacs-agent--claude-plan
  :select #'romacs-agent--claude-select
  :argv #'romacs-agent--claude-argv
  :input #'romacs-agent--claude-user
  :interrupt #'romacs-agent--claude-interrupt
  :respond #'romacs-agent--claude-respond
  :events #'romacs-agent--claude-events
  :input-events #'romacs-agent--claude-input-events
  ;; A turn the harness's end cut short goes on (Claude Code's own switch).
  :resume-environment '("CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1")
  :native-id-p #'romacs-agent--uuid-p
  :history '(:files romacs-agent-history--claude-files :keep romacs-agent-history--claude-keep
             :describe romacs-agent-history--claude-describe :events romacs-agent-history--claude-events))

(provide 'romacs-agent-claude)
;;; romacs-agent-claude.el ends here
