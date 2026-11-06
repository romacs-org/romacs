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
                                             "--reply-timeout" (number-to-string romacs-agent-reply-timeout))))))
    (append (romacs-agent--environment-words session)
            (list (plist-get instance :program) "-p" "--output-format" "stream-json"
                  "--input-format" "stream-json" "--verbose" "--include-partial-messages"
                  ;; Without it the stream carries thinking blocks with no text.
                  "--thinking-display" "summarized"
                  ;; The harness's own approvals come to this session's UI.
                  "--permission-prompt-tool" "stdio"
                  "--mcp-config" (expand-file-name "mcp.json" dir))
            (if resume (list "--resume" (romacs-agent--native-id session))
              (list "--session-id" (romacs-agent-id session)))
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

(defun romacs-agent--claude-user (_session text)
  "The harness's message of TEXT, with the images attached to it as content:
the model is given them, and no file is named for it to open."
  (let ((images (seq-filter #'romacs-agent-image-file-p romacs-agent--message-images)))
    (romacs-mcp--object
     "type" "user"
     "message" (romacs-mcp--object
                "role" "user"
                "content" (if images
                              (vconcat (list (romacs-mcp--object "type" "text" "text" text))
                                       (mapcar #'romacs-agent--claude-image images))
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

(defun romacs-agent--claude-events (session line)
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
         ("status" (romacs-agent--claude-status line))
         ("compact_boundary" (setf (romacs-agent-compacting session) t) nil)
         (_ nil)))
      ("stream_event"
       (let* ((event (gethash "event" line))
              (kind (and event (gethash "type" event))))
         (pcase kind
           ("message_start"
            (setf (romacs-agent-message session) (gethash "id" (gethash "message" event)))
            nil)
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
         (when (equal (gethash "subtype" request) "can_use_tool")
           (puthash (gethash "request_id" line) request (romacs-agent-approvals session))
           (list (list :type 'approval :request-id (gethash "request_id" line)
                       :tool (gethash "tool_name" request) :input (gethash "input" request)
                       :description (gethash "description" request))))))
      ("control_response"
       (let* ((response (gethash "response" line)) (id (gethash "request_id" response)))
         (cond ((and id (romacs-agent--select-answered session id (equal (gethash "subtype" response) "success"))))
               ((and id (equal id (romacs-agent-models-request session)))
                (setf (romacs-agent-models-request session) nil)
                (when (hash-table-p (gethash "response" response))
                  (romacs-agent--keep-models session (romacs-agent--claude-models (gethash "response" response)))))
               ((and id (gethash id (romacs-agent-approvals session)))
                (remhash id (romacs-agent-approvals session))))
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

(defun romacs-agent--claude-refresh-models (session)
  "Ask SESSION's Claude Code for its models, as its own SDK does at the
start: its answer comes with the other control responses."
  (let ((id (format "romacs-models-%d" (romacs-agent-request-id session))))
    (setf (romacs-agent-models-request session) id)
    (romacs-agent--line session (romacs-mcp--object "type" "control_request" "request_id" id
                                                    "request" (romacs-mcp--object "subtype" "initialize")))))

;; A record's image files are trusted as what the user attached: the journal
;; is this user's private state, whoever can rewrite it can read those files
;; anyway, and a recalled image shows in the tray before anything is sent.
(defun romacs-agent--claude-input-events (session line)
  "The events of LINE, written to SESSION's harness: a user's prompt, or
an approval answered."
  (pcase (gethash "type" line)
    ("user" (list (list :type 'input :text (romacs-agent--text-of (gethash "content" (gethash "message" line)))
                        :images (append (gethash "images" line) nil))))
    ("control_response"
     (let ((id (gethash "request_id" (gethash "response" line))))
       (remhash id (romacs-agent-approvals session))
       (list (list :type 'answered :request-id id
                   :behavior (gethash "behavior" (gethash "response" (gethash "response" line)))))))
    (_ nil)))

(defun romacs-agent--claude-interrupt (session)
  (romacs-mcp--object "type" "control_request"
                      "request_id" (format "romacs-interrupt-%d" (romacs-agent-request-id session))
                      "request" (romacs-mcp--object "subtype" "interrupt")))

(defun romacs-agent--claude-respond (session request-id allow message)
  "The control_response answering approval REQUEST-ID: the tool as asked
when ALLOW, else denied with MESSAGE."
  (let ((request (gethash request-id (romacs-agent-approvals session))))
    (romacs-mcp--object
     "type" "control_response"
     "response" (romacs-mcp--object
                 "subtype" "success" "request_id" request-id
                 "response" (if allow
                                (romacs-mcp--object "behavior" "allow"
                                                    "updatedInput" (or (gethash "input" request) (romacs-mcp--object)))
                              (romacs-mcp--object "behavior" "deny"
                                                  "message" (or message "Denied by the user")))))))

(romacs-agent-define-harness 'claude
  :display "Claude Code" :icon "✳" :t3-driver "claudeAgent"
  :config-variable "CLAUDE_CONFIG_DIR" :config-default "~/.claude"
  :program-option 'romacs-agent-claude-program :arguments-option 'romacs-agent-claude-arguments
  ;; What Claude Code sets for the commands it runs (a parent session's).
  :scrub '("CLAUDECODE" "CLAUDE_AGENT_SDK_VERSION" "CLAUDE_CODE_CHILD_SESSION" "CLAUDE_CODE_ENTRYPOINT"
           "CLAUDE_CODE_EXECPATH" "CLAUDE_CODE_MESSAGING_SOCKET" "CLAUDE_CODE_MESSAGING_TOKEN"
           "CLAUDE_CODE_SESSION_ATTENDED" "CLAUDE_CODE_SESSION_ID" "CLAUDE_EFFORT" "CLAUDE_PID")
  :lifetime 'persistent
  :refresh-models #'romacs-agent--claude-refresh-models
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
