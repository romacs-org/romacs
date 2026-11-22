;;; romacs-agent-codex-history.el --- Read Codex rollouts -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Discover Codex rollouts and decode their recorded conversations.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent-transcript)
(require 'romacs-agent-record)
(require 'romacs-mcp)
(declare-function romacs-agent--text-of "romacs-agent" (content))
(declare-function romacs-agent--uuid-p "romacs-agent" (id))
(declare-function romacs-agent-incoming-of "romacs-agent-tools" (text))

(defun romacs-agent-codex-history--field (object &rest keys)
  "OBJECT's value at KEYS, each a key of the object the last one gave."
  (while (and keys (hash-table-p object)) (setq object (gethash (pop keys) object)))
  (and (null keys) object))

(defun romacs-agent-codex-history--keep (object)
  "Whether Codex's rollout line OBJECT is one its history reads."
  (pcase (gethash "type" object)
    ((or "session_meta" "turn_context") t)
    ("response_item"
     (member (romacs-agent-codex-history--field object "payload" "type")
             '("message" "function_call" "function_call_output" "custom_tool_call" "custom_tool_call_output"
               "local_shell_call" "web_search_call")))
    ("event_msg"
     (pcase (romacs-agent-codex-history--field object "payload" "type")
       ((or "user_message" "task_complete" "token_count") t)
       ("item_completed" (equal (romacs-agent-codex-history--field object "payload" "item" "type") "UserMessage"))))))

(defun romacs-agent-codex-history--files (dir limit)
  "Codex's rollouts under DIR, newest days first, until LIMIT are found:
\(PATH SIZE MTIME)."
  (let ((files nil) (count 0))
    (cl-flet ((under (parent) (and (file-directory-p parent) (reverse (directory-files parent t "\\`[0-9]+\\'")))))
      (catch 'enough
        (dolist (year (under (expand-file-name "sessions" dir)))
          (dolist (month (under year))
            (dolist (day (under month))
              (dolist (entry (directory-files-and-attributes day t "\\`rollout-.*\\.jsonl\\'" t))
                (cl-incf count)
                (push (list (car entry) (file-attribute-size (cdr entry))
                            (float-time (file-attribute-modification-time (cdr entry))))
                      files))
              (when (>= count limit) (throw 'enough nil)))))))
    files))

(defun romacs-agent-codex-history--find (dir id)
  "Codex's rollout of thread ID under DIR, whatever day it began, or nil."
  (car (file-expand-wildcards (expand-file-name (format "sessions/*/*/*/rollout-*-%s.jsonl" id) dir))))

(defun romacs-agent-codex-history--prompt (payload)
  "The user's typed text in a Codex event PAYLOAD, or nil."
  (pcase (gethash "type" payload)
    ("user_message" (romacs-agent--value (gethash "message" payload)))
    ("item_completed" (romacs-agent--text-of (gethash "content" (gethash "item" payload))))))

(defun romacs-agent-codex-history--describe (objects path)
  "Describe a Codex rollout at PATH from its head OBJECTS."
  (let ((thread nil) (cwd nil) (title nil))
    (cl-loop for line in objects
             for payload = (gethash "payload" line)
             when (hash-table-p payload) do
             (pcase (gethash "type" line)
               ("session_meta" (setq thread (romacs-agent--value (gethash "id" payload))
                                     cwd (romacs-agent--value (gethash "cwd" payload))))
               ("event_msg" (setq title (or title (romacs-agent-transcript-title (romacs-agent-codex-history--prompt payload))))))
             until title)
    (setq thread (or thread (and (string-match "\\([0-9a-f]\\{8\\}-[0-9a-f-]\\{27\\}\\)\\.jsonl\\'" path)
                                 (match-string 1 path))))
    (when (and (romacs-agent--uuid-p thread) cwd)
      (list :harness 'codex :id thread :cwd cwd :title title))))

(defun romacs-agent-codex-history--input (payload)
  (let ((arguments (gethash "arguments" payload)) (input (gethash "input" payload)))
    (cond ((stringp arguments)
           (let ((object (ignore-errors (json-parse-string arguments :object-type 'hash-table
                                                           :null-object :null :false-object :false))))
             (if (hash-table-p object)
                 (progn (when (and (gethash "cmd" object) (not (gethash "command" object)))
                          (puthash "command" (gethash "cmd" object) object))
                        object)
               (romacs-mcp--object "code" arguments))))
          ((stringp input) (romacs-mcp--object "code" input))
          (t nil))))

(defun romacs-agent-codex-history--output (output)
  (cond ((stringp output) output)
        ((vectorp output) (romacs-agent--text-of output))
        ((hash-table-p output) (or (gethash "output" output) (romacs-agent--text-of (or (gethash "content" output) ""))))
        (t "")))

(defun romacs-agent-codex-history--response (type payload message)
  "Events of a Codex response of TYPE, PAYLOAD and assistant MESSAGE number."
  (pcase type
    ("message"
     (when (equal (gethash "role" payload) "assistant")
       (let ((id (format "history:%d" message))
             (text (romacs-agent--text-of (gethash "content" payload))))
         (unless (string-empty-p text)
           (list (list :type 'delta :text (concat text "\n") :message id)
                 (list :type 'message-end :message id))))))
    ((or "function_call" "custom_tool_call")
     (let ((name (gethash "name" payload)))
       (list (list :type 'tool :state 'started :id (gethash "call_id" payload)
                   :name (if (member name '("exec_command" "shell")) "Bash" name)
                   :input (romacs-agent-codex-history--input payload)))))
    ((or "function_call_output" "custom_tool_call_output")
     (list (list :type 'tool :state 'completed :id (gethash "call_id" payload)
                 :output (romacs-agent-codex-history--output (gethash "output" payload)))))
    (_ (let ((id (or (romacs-agent--value (gethash "call_id" payload))
                     (romacs-agent--value (gethash "id" payload)))))
         (list (list :type 'tool :state 'started :id id :name type :input nil)
               (list :type 'tool :state 'completed :id id :output ""))))))

(defun romacs-agent-codex-history--usage (payload)
  "What the token usage in PAYLOAD says about the conversation, or nil."
  (let* ((info (gethash "info" payload))
         (usage (and (hash-table-p info) (gethash "last_token_usage" info))))
    (when (hash-table-p usage)
      (list :type 'meta :context-used (gethash "input_tokens" usage)
            :context-window (romacs-agent--value (gethash "model_context_window" info))))))

(defun romacs-agent-codex-history--turn-start (payload)
  "Events beginning a turn from PAYLOAD, or nil."
  (when-let* ((prompt (romacs-agent-codex-history--prompt payload)))
    (if-let* ((incoming (romacs-agent-incoming-of prompt)))
        (mapcar (lambda (note) (append (list :type 'arrival) note)) incoming)
      (when (romacs-agent-transcript-title prompt)
        (list (list :type 'input :text prompt))))))

(defun romacs-agent-codex-history--events (objects)
  "A Codex rollout's OBJECTS as chat events."
  (let ((events nil) (turn nil) (last nil) (model nil) (messages 0))
    (cl-flet ((close ()
                (when turn
                  (push (list :type 'done :duration (romacs-agent-transcript-duration turn last)) events)
                  (setq turn nil))))
      (dolist (line objects)
        (when-let* ((payload (gethash "payload" line))
                    ((hash-table-p payload)))
          (let ((kind (gethash "type" line))
                (type (gethash "type" payload))
                (time (gethash "timestamp" line)))
            (setq last (or time last))
            (pcase kind
              ("turn_context"
               (let ((name (romacs-agent--value (gethash "model" payload))))
                 (when (and (stringp name) (not (equal name model)))
                   (setq model name)
                   (push (list :type 'meta :model name) events))))
              ("event_msg"
               (pcase type
                 ("task_complete" (close))
                 ("token_count"
                  (when-let* ((usage (romacs-agent-codex-history--usage payload)))
                    (push usage events)))
                 (_ (when-let* ((begun (romacs-agent-codex-history--turn-start payload)))
                      (close)
                      (setq turn time)
                      (dolist (event begun) (push event events))))))
              ("response_item"
               (when (and (equal type "message") (equal (gethash "role" payload) "assistant"))
                 (cl-incf messages))
               (dolist (event (romacs-agent-codex-history--response type payload messages))
                 (push event events)))))))
      (close))
    (nreverse events)))

(provide 'romacs-agent-codex-history)
;;; romacs-agent-codex-history.el ends here
