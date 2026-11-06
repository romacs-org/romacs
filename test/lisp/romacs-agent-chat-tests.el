;;; romacs-agent-chat-tests.el --- A chat buffer's commands -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-ui))

(defmacro romacs-agent-chat-tests--with (transcript draft &rest body)
  "BODY in a chat buffer of TRANSCRIPT, then a composer holding DRAFT."
  (declare (indent 2))
  `(with-temp-buffer
     (romacs-agent-chat-mode)
     (let ((inhibit-read-only t))
       (insert ,transcript)
       (setq romacs-agent-ui--end (point-marker))
       (insert ,draft))
     (goto-char (point-min))
     ,@body))

(defun romacs-agent-chat-tests--prompt (text)
  (propertize (concat "> " text "\n") 'romacs-agent-turn t))

(ert-deftest romacs-agent-chat-no-session-has-no-turns ()
  ;; The mode on a buffer no session shows: `e' and `w' say so.
  (with-temp-buffer
    (romacs-agent-chat-mode)
    (let ((inhibit-read-only t)) (insert "You\nsome text\n"))
    (goto-char (point-min))
    (should (equal (cadr (should-error (romacs-agent-chat-edit-turn) :type 'user-error)) "No session here"))
    (should-error (romacs-agent-chat-copy) :type 'user-error)))

(ert-deftest romacs-agent-chat-edit-before-any-turn-keeps-the-draft ()
  (romacs-agent-chat-tests--with "Resumed from an earlier session\n" "my draft"
    (should (equal (cadr (should-error (romacs-agent-chat-edit-turn) :type 'user-error)) "No turn here"))
    (should (equal (romacs-agent-ui--composer-text) "my draft")))
  (romacs-agent-chat-tests--with "" "my draft"
    (should-error (romacs-agent-chat-edit-turn) :type 'user-error)
    (should (equal (romacs-agent-ui--composer-text) "my draft"))))

(ert-deftest romacs-agent-chat-edit-takes-the-prompt-at-point ()
  (romacs-agent-chat-tests--with
      (concat "Preamble\n" (romacs-agent-chat-tests--prompt "first") "reply one\n"
              (romacs-agent-chat-tests--prompt "second") "reply two\n")
      "my draft"
    (dolist (at '(("reply one" . "first") ("> first" . "first") ("reply two" . "second") ("> second" . "second")))
      (goto-char (point-min))
      (search-forward (car at))
      (backward-char 1)
      (let ((inhibit-read-only t)) (romacs-agent-chat-edit-turn))
      (should (equal (romacs-agent-ui--composer-text) (cdr at))))
    ;; Before the first prompt, in a transcript that has some: none is at point.
    (goto-char (point-min))
    (should-error (romacs-agent-chat-edit-turn) :type 'user-error)))

(ert-deftest romacs-agent-chat-a-window-start-stays-at-the-transcript ()
  ;; A window starting at the transcript's end stays where it is as text
  ;; comes in, while point at the composer follows.
  (romacs-agent-chat-tests--with "" ""
    (save-window-excursion
      (set-window-buffer (selected-window) (current-buffer))
      (set-window-start (selected-window) (point-min))
      (goto-char romacs-agent-ui--end)
      (set-window-point (selected-window) (point))
      (romacs-agent-ui--insert-at romacs-agent-ui--end "> first\n")
      (should (= (window-start) (point-min)))
      (should (= (window-point) (point-max)))
      (should (= romacs-agent-ui--end (point-max))))))

(provide 'romacs-agent-chat-tests)
;;; romacs-agent-chat-tests.el ends here
