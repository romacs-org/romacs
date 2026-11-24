;;; romacs-agent-log-undo-tests.el --- The chat's writes keep no undo -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent-log))

(ert-deftest romacs-agent-log-writing-keeps-no-undo ()
  ;; The chat's own writes are no edit of the user's: none is undone.
  (with-temp-buffer
    (insert "transcript")
    (buffer-enable-undo)
    (insert "typed")
    (let ((undo buffer-undo-list))
      (should (consp undo))
      (romacs-agent-log--writing (insert " written"))
      (should (eq buffer-undo-list undo))
      (romacs-agent-log--writing (put-text-property 1 2 'face 'bold))
      (should (eq buffer-undo-list undo)))))

;;; romacs-agent-log-undo-tests.el ends here
