;;; romacs-agent-claude-history.el --- Read Claude Code transcripts -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Code:

(require 'cl-lib)
(require 'romacs-agent-transcript)
(require 'romacs-agent-record)
(declare-function romacs-agent--text-of "romacs-agent" (content))
(declare-function romacs-agent--claude-incoming "romacs-agent-claude" (text))
(declare-function romacs-agent--claude-user-events "romacs-agent-claude" (line parent compacting))
(declare-function romacs-agent--claude-message-events "romacs-agent-claude" (message parent streamed))

(defun romacs-agent-claude-history--tool-result-p (content)
  "Whether CONTENT contains a tool result."
  (and (vectorp content)
       (cl-some (lambda (block)
                  (and (hash-table-p block) (equal (gethash "type" block) "tool_result")))
                content)))

(defun romacs-agent-claude-history--prompt (content)
  "The user's typed text in a Claude Code message CONTENT, or nil (a tool
result, an injected context)."
  (cond ((stringp content) (and (romacs-agent-transcript-title content) content))
        ((vectorp content)
         (unless (romacs-agent-claude-history--tool-result-p content)
           (let ((text (romacs-agent--text-of content)))
             (and (romacs-agent-transcript-title text) text))))))

(defun romacs-agent-claude-history--keep (object)
  "Whether Claude Code's transcript line OBJECT is a message's."
  (member (gethash "type" object) '("user" "message" "assistant")))

(defun romacs-agent-claude-history--files (dir _limit)
  "Claude Code's transcripts under DIR: (PATH SIZE MTIME)."
  (let ((projects (expand-file-name "projects" dir)) (files nil))
    (when (file-directory-p projects)
      (dolist (project (directory-files projects t "\\`[^.]"))
        (when (file-directory-p project)
          (dolist (entry (directory-files-and-attributes project t "\\`[0-9a-f-]\\{36\\}\\.jsonl\\'" t))
            (push (list (car entry) (file-attribute-size (cdr entry))
                        (float-time (file-attribute-modification-time (cdr entry))))
                  files)))))
    files))

(defun romacs-agent-claude-history--find (dir id)
  "Claude Code's transcript of session ID under DIR, in whichever
project it was run, or nil."
  (car (file-expand-wildcards (expand-file-name (format "projects/*/%s.jsonl" id) dir))))

(defun romacs-agent-claude-history--describe (objects path)
  "A Claude Code transcript's description from its head's OBJECTS, its
:title nil while they name no prompt; nil while they name no directory."
  (let ((cwd nil) (title nil))
    (cl-loop for line in objects
             when (and (equal (gethash "type" line) "user") (not (eq (gethash "isSidechain" line) t))) do
             (setq cwd (or cwd (romacs-agent--value (gethash "cwd" line))))
             (unless (or title (eq (gethash "isMeta" line) t))
               (setq title (romacs-agent-transcript-title
                            (romacs-agent-claude-history--prompt (gethash "content" (gethash "message" line))))))
             until title)
    (when cwd
      (list :harness 'claude :id (file-name-base path) :cwd cwd :title title))))

(defun romacs-agent-claude-history--turn-start (content mode)
  "Events beginning a turn from CONTENT in MODE, or nil."
  (unless (romacs-agent-claude-history--tool-result-p content)
    (if-let* ((incoming (romacs-agent--claude-incoming (romacs-agent--text-of content))))
        (mapcar (lambda (note) (append (list :type 'arrival) note)) incoming)
      (when-let* ((prompt (romacs-agent-claude-history--prompt content)))
        (list (list :type 'meta :permission-mode mode)
              (list :type 'input :text prompt))))))

(defun romacs-agent-claude-history--events (objects)
  "A Claude Code transcript's OBJECTS as chat events."
  (let ((events nil) (turn nil) (last nil) (model nil))
    (cl-flet ((close ()
                (when turn
                  (push (list :type 'done :duration (romacs-agent-transcript-duration turn last))
                        events)
                  (setq turn nil))))
      (dolist (line objects)
        (unless (or (eq (gethash "isSidechain" line) t) (eq (gethash "isMeta" line) t))
          (pcase (gethash "type" line)
            ((and "user" (guard (eq (gethash "isCompactSummary" line) t)))
             (push (list :type 'compacted :text (romacs-agent--text-of (gethash "content" (gethash "message" line))))
                   events))
            ("user"
             (let* ((content (gethash "content" (gethash "message" line)))
                    (begun (romacs-agent-claude-history--turn-start content (gethash "permissionMode" line))))
               (cond (begun
                      (close)
                      (setq turn (gethash "timestamp" line))
                      (dolist (event begun) (push event events)))
                     ((vectorp content)
                      (dolist (event (romacs-agent--claude-user-events line (romacs-agent--value (gethash "parent_tool_use_id" line)) nil))
                        (when (eq (plist-get event :type) 'tool) (push event events)))))))
            ("assistant"
             (let* ((message (gethash "message" line))
                    (name (gethash "model" message))
                    (parent (romacs-agent--value (gethash "parent_tool_use_id" line))))
               (when (and (stringp name) (not (equal name model)) (not (string-prefix-p "<" name)))
                 (setq model name)
                 (push (list :type 'meta :model name) events))
               (dolist (event (romacs-agent--claude-message-events message parent nil))
                 (push event events)))))
          (setq last (or (gethash "timestamp" line) last))))
      (close))
    (nreverse events)))

(provide 'romacs-agent-claude-history)
;;; romacs-agent-claude-history.el ends here
