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

;;; The composer edits as any buffer; the transcript's keys are its text's

(defmacro romacs-agent-chat-tests--sealed (draft &rest body)
  "BODY in a chat buffer of a sealed transcript (a line and a link),
then a composer holding DRAFT, point at the composer's end."
  (declare (indent 1))
  `(with-temp-buffer
     (romacs-agent-chat-mode)
     (let ((inhibit-read-only t))
       (insert (romacs-agent-ui--sealed "> asked\nanswered ")
               (romacs-agent-ui--sealed (propertize "link" 'keymap romacs-agent-ui--link-map))
               (romacs-agent-ui--sealed "\n"))
       (setq romacs-agent-ui--end (point-marker))
       (insert ,draft))
     ,@body))

(defun romacs-agent-chat-tests--key (key position)
  "The command KEY runs with point at POSITION."
  (save-excursion
    (goto-char position)
    (let ((command (key-binding (kbd key) t nil (point))))
      (or (command-remapping command (point)) command))))

(ert-deftest romacs-agent-chat-transcript-keys-are-its-text-s ()
  (romacs-agent-chat-tests--sealed "draft"
    (let ((transcript 3) (composer (point-max)))
      (should (eq (romacs-agent-chat-tests--key "DEL" transcript) #'scroll-down-command))
      (should (eq (romacs-agent-chat-tests--key "TAB" transcript) #'romacs-agent-chat-toggle))
      (should (eq (romacs-agent-chat-tests--key "RET" transcript) #'romacs-agent-chat-toggle))
      (should (eq (romacs-agent-chat-tests--key "n" transcript) #'romacs-agent-chat-next-turn))
      (should (eq (romacs-agent-chat-tests--key "a" transcript) #'romacs-agent-chat-type))
      ;; The composer, its start included, has the buffer's own keys.
      (dolist (position (list composer (marker-position romacs-agent-ui--end)))
        (should (eq (romacs-agent-chat-tests--key "DEL" position) #'delete-backward-char))
        (should (eq (romacs-agent-chat-tests--key "n" position) #'self-insert-command))
        (should (eq (romacs-agent-chat-tests--key "M-DEL" position) #'backward-kill-word))
        (should (eq (romacs-agent-chat-tests--key "RET" position) #'romacs-agent-chat-return)))
      ;; A link keeps its keys and falls back to the transcript's.
      (let ((link (string-search "link" (buffer-string))))
        (should (eq (romacs-agent-chat-tests--key "RET" (1+ link)) #'romacs-agent-chat-open-task))
        (should (eq (romacs-agent-chat-tests--key "n" (1+ link)) #'romacs-agent-chat-next-turn))))))

(ert-deftest romacs-agent-chat-composer-deletes-its-region ()
  (romacs-agent-chat-tests--sealed "keep this"
    (let ((transient-mark-mode t))
      (goto-char (point-max))
      (push-mark (- (point-max) 4) t t)
      (call-interactively #'delete-backward-char)
      (should (equal (buffer-substring-no-properties romacs-agent-ui--end (point-max)) "keep ")))))

(ert-deftest romacs-agent-chat-word-kills-stop-at-the-composer ()
  (romacs-agent-chat-tests--sealed "foo bar"
    (let ((transcript (buffer-substring-no-properties (point-min) romacs-agent-ui--end)))
      (goto-char (point-max))
      (dotimes (_ 3) (backward-kill-word 1))
      (should (= (point) romacs-agent-ui--end))
      (should (equal (buffer-substring-no-properties romacs-agent-ui--end (point-max)) ""))
      (should (equal (buffer-substring-no-properties (point-min) romacs-agent-ui--end) transcript)))))

(ert-deftest romacs-agent-chat-typing-in-the-transcript-types-in-the-composer ()
  (romacs-agent-chat-tests--sealed "draft"
    (goto-char 3)
    (let ((last-command-event ?x)) (romacs-agent-chat-type 2))
    (should (equal (buffer-substring-no-properties romacs-agent-ui--end (point-max)) "draftxx"))
    (goto-char 3)
    (romacs-agent-chat-newline)
    (should (equal (buffer-substring-no-properties romacs-agent-ui--end (point-max)) "draftxx\n"))))

(ert-deftest romacs-agent-chat-typed-text-takes-no-transcript-keys ()
  (romacs-agent-chat-tests--sealed ""
    (goto-char (point-max))
    (let ((last-command-event ?n)) (self-insert-command 1))
    (should-not (get-text-property (1- (point-max)) 'keymap))
    (should (eq (romacs-agent-chat-tests--key "n" (1- (point-max))) #'self-insert-command))))
