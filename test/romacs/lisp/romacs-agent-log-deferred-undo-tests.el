;;; romacs-agent-log-deferred-undo-tests.el --- Undo in the composer across chat output -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The chat's output lands in its own buffer: undo in the composer goes
;; on as if none came.

;;; Code:

(require 'cl-lib)
(require 'ert)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent-log))

(defmacro romacs-agent-log-deferred-undo-tests--with-chat (&rest body)
  "Run BODY in a chat's composer, undo enabled; `chat' is the chat."
  (declare (indent 0))
  `(with-temp-buffer
     (romacs-agent-log-mode)
     (setq romacs-agent-log--session (romacs-agent--make :id "undo" :harness 'claude :state 'running)
           romacs-agent-log--end (copy-marker (point-min) t))
     (romacs-agent-log--separator t)
     (let ((chat (current-buffer)))
       (with-current-buffer (romacs-agent-log-composer-buffer chat)
         ,@body))))

(ert-deftest romacs-agent-log-selective-undo-continues-after-output ()
  (romacs-agent-log-deferred-undo-tests--with-chat
    (dolist (text '("a" "b" "outside"))
      (insert text)
      (undo-boundary))
    (let ((transient-mark-mode t)
          (last-command 'self-insert-command)
          (this-command 'undo))
      (goto-char 3)
      (set-mark (point-min))
      (activate-mark)
      (call-interactively #'undo)
      (should undo-in-region)
      (should (equal (buffer-string) "aoutside"))
      (undo-boundary)
      (with-current-buffer chat (romacs-agent-log--stream-root 'text "Streamed output"))
      (let ((last-command 'undo))
        (call-interactively #'undo)
        (should (equal (buffer-string) "outside"))))))

(ert-deftest romacs-agent-log-undo-after-worker-activity-appears ()
  (romacs-agent-log-deferred-undo-tests--with-chat
    (insert "draft")
    (undo-boundary)
    (with-current-buffer chat
      (cl-letf (((symbol-function 'romacs-agent-children-rows)
                 (lambda (_) '((:subagent t :state "running" :tool "worker" :title "Reading")))))
        (romacs-agent-log--activity-refresh)))
    (let ((chrome (with-current-buffer chat (buffer-string)))
          (last-command 'self-insert-command)
          (this-command 'undo))
      (run-hooks 'pre-command-hook)
      (call-interactively #'undo)
      (should (equal (buffer-string) ""))
      (should (equal-including-properties (with-current-buffer chat (buffer-string)) chrome))
      (should (with-current-buffer chat (romacs-agent-log-activity-bounds))))))

;;; romacs-agent-log-deferred-undo-tests.el ends here
