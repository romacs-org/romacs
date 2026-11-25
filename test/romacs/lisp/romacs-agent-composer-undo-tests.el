;;; romacs-agent-composer-undo-tests.el --- Undo after chat output -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-log))

(ert-deftest romacs-agent-composer-undo-preserves-streamed-transcript ()
  (let* ((romacs-agent-log--chats (make-hash-table :test #'equal))
         (session (romacs-agent--make :id "composer-undo" :harness 'claude :state 'running))
         (buffer (romacs-agent-log-chat session)))
    (unwind-protect
        (with-current-buffer buffer
          (romacs-agent-log--on-event session '(:type delta :text "Reply"))
          (goto-char (point-max))
          (insert "draft")
          (undo-boundary)
          (insert " tail")
          (undo-boundary)
          (romacs-agent-log--on-event session '(:type delta :text " continues streaming"))
          (let ((transcript (buffer-substring-no-properties (point-min) romacs-agent-log--end))
                (last-command 'self-insert-command)
                (this-command 'undo))
            (should (equal transcript "Reply continues streaming\n"))
            (should (equal (buffer-substring-no-properties romacs-agent-log--input (point-max))
                           "draft tail"))
            (call-interactively #'undo)
            (should (equal (list (buffer-substring-no-properties romacs-agent-log--input (point-max))
                                 (buffer-substring-no-properties (point-min) romacs-agent-log--end))
                           (list "draft" transcript)))))
      (kill-buffer buffer))))
