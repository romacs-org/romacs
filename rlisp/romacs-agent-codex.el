;;; romacs-agent-codex.el --- Codex, as Romacs runs it  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Codex's driver (`romacs-agent-define-harness'): one `codex exec --json'
;; run per turn, its prompt on stdin, going on with its thread by
;; `exec resume'.  An interrupt ends the run.

;;; Code:

(require 'romacs-agent)

(defun romacs-agent--codex-image-words (session)
  "Codex's arguments for the images attached to this turn: it is given them
on its command line, and does not open a file a prompt names.  Attached,
never read from the prompt: `--image' uploads the file."
  (ignore session)
  (mapcan (lambda (file) (list "--image" file))
          (seq-filter #'romacs-agent-image-file-p romacs-agent--turn-images)))

(defun romacs-agent--codex-argv (session resume)
  "`codex exec''s command line for one turn of SESSION, its prompt on
stdin; RESUME non-nil continues its thread."
  (let ((instance (romacs-agent--instance (romacs-agent-provider session) 'codex))
        (thread (romacs-agent-harness-session session)))
    (append (romacs-agent--environment-words session)
            (list (plist-get instance :program) "exec")
            (when (and resume thread) (list "resume" thread))
            (romacs-agent--codex-image-words session)
            (list "--json" "--skip-git-repo-check")
            ;; Romacs's tools, through the reconnecting relay.
            (romacs-agent--codex-mcp-words session)
            (let ((options (romacs-agent-options session)))
              (append (when-let* ((model (plist-get options :model))) (list "-m" model))
                      (when-let* ((effort (plist-get options :effort)))
                        (list "-c" (format "model_reasoning_effort=%S" effort)))))
            (plist-get instance :arguments)
            (list "-"))))

(defun romacs-agent--codex-mcp-words (session)
  (list "-c" (format "mcp_servers.romacs.command=%S" (romacs-agent--program "romacs-mcp"))
        "-c" (format "mcp_servers.romacs.args=[%s]"
                     (mapconcat (lambda (word) (format "%S" word))
                                (list "--socket" (romacs-agent--route session) "--reconnect"
                                      "--session" (romacs-agent-id session)
                                      "--reply-timeout" (number-to-string romacs-agent-reply-timeout))
                                ","))))

(defun romacs-agent--codex-tool (item)
  "An item's tool name and input, for the tool block."
  (pcase (gethash "type" item)
    ("command_execution" (list "Bash" (romacs-mcp--object "command" (gethash "command" item))))
    ("mcp_tool_call" (list (format "mcp__%s__%s" (gethash "server" item) (gethash "tool" item))
                           (gethash "arguments" item)))
    ("file_change" (list "Edit" (romacs-mcp--object "changes" (gethash "changes" item))))
    (type (list type nil))))

(defun romacs-agent--codex-output (item)
  (let ((output (or (gethash "aggregated_output" item) (gethash "result" item) (gethash "error" item) "")))
    (cond ((stringp output) output)
          ((and (hash-table-p output) (gethash "content" output)) (romacs-agent--text-of (gethash "content" output)))
          (t (romacs-agent--json output)))))

(defun romacs-agent--codex-item (session item completed)
  "The events of ITEM, started or COMPLETED, in SESSION's current turn."
  (let ((type (gethash "type" item))
        (id (format "%d:%s" (romacs-agent-turns session) (gethash "id" item))))
    (pcase type
      ("agent_message"
       (when completed
         (setf (romacs-agent-reply session) (gethash "text" item))
         (list (list :type 'delta :text (concat (gethash "text" item) "\n") :message id)
               (list :type 'message-end :message id))))
      ("reasoning" (when completed (list (list :type 'thinking :text (or (gethash "text" item) "")))))
      ((or "todo_list" "error") nil)
      (_
       (let* ((started (unless (gethash id (romacs-agent-tools session))
                         (puthash id t (romacs-agent-tools session))
                         (pcase-let ((`(,name ,input) (romacs-agent--codex-tool item)))
                           (list (list :type 'tool :state 'started :id id :name name :input input)))))
              (failed (or (member (gethash "status" item) '("failed" "declined"))
                          (and (integerp (gethash "exit_code" item)) (/= 0 (gethash "exit_code" item))))))
         (append started
                 (when completed
                   (list (list :type 'tool :state (if failed 'failed 'completed) :id id
                               :output (romacs-agent--codex-output item))))))))))

(defun romacs-agent--codex-events (session line)
  "The events of LINE, one of `codex exec --json''s objects."
  (pcase (gethash "type" line)
    ("thread.started" (list (list :type 'session :harness-session (gethash "thread_id" line)
                                  :model (plist-get (romacs-agent-options session) :model))))
    ("turn.started" (setf (romacs-agent-reply session) nil) nil)
    ("item.started" (romacs-agent--codex-item session (gethash "item" line) nil))
    ("item.updated" nil)
    ("item.completed" (romacs-agent--codex-item session (gethash "item" line) t))
    ("turn.completed" (list (list :type 'done :text (romacs-agent-reply session) :usage (gethash "usage" line))))
    ("turn.failed" (list (list :type 'done :error t
                               :text (let ((error (gethash "error" line)))
                                       (or (and (hash-table-p error) (gethash "message" error)) "turn failed")))))
    ("error" (list (list :type 'error :text (gethash "message" line))))
    (_ (list (list :type 'unknown :raw line)))))

(romacs-agent-define-harness 'codex
  :display "Codex" :icon "◎" :t3-driver "codex"
  :config-variable "CODEX_HOME" :config-default "~/.codex"
  :program-option 'romacs-agent-codex-program :arguments-option 'romacs-agent-codex-arguments
  ;; What Codex sets for the commands it runs (codex-cli 0.164, read from
  ;; a child's environment).
  :scrub '("CODEX_APPLY_PATCH_PRESERVE_LINE_ENDINGS" "CODEX_CI" "CODEX_SANDBOX_NETWORK_DISABLED"
           "CODEX_SESSION_ID" "CODEX_THREAD_ID" "CODEX_TOOL_CALL_ID" "CODEX_VERSION")
  :lifetime 'per-turn
  :argv #'romacs-agent--codex-argv
  :interrupt (lambda (_session) 'signal)
  :events #'romacs-agent--codex-events
  :native-id-p #'romacs-agent--uuid-p
  :history '(:files romacs-agent-history--codex-files :keep romacs-agent-history--codex-keep
             :describe romacs-agent-history--codex-describe :events romacs-agent-history--codex-events))

(provide 'romacs-agent-codex)
;;; romacs-agent-codex.el ends here
