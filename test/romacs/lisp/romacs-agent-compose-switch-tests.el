;;; romacs-agent-compose-switch-tests.el --- Buffers switched to from a composer -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The composer's window is dedicated.  A buffer switched to, or a file
;; found, from it shows, as a session does it: through Romacs's own
;; `switch-to-buffer' (lisp/completion/romacs-buffer-switch.el) and
;; `find-file' (lisp/romacs-find-file.el), which a session loads in
;; place of GNU's.  Loading them here redefines both for this file alone.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-log))

;; The session's, as startup.rs loads them; its `find-file' only where
;; GNU's visiting is not loaded, so made to think it is not.
(load "romacs-buffer-switch" nil t)
;; What its visitor reads, as lisp/session/romacs-startup.el has them.
(defvar romacs-gui-preserve-mode-line nil)
(defvar romacs-gui-visits nil)
(cl-letf (((symbol-function 'find-file-noselect-1) nil))
  (load "romacs-find-file" nil t))

(defmacro romacs-agent-compose-switch-tests--from-composer (&rest body)
  "Run BODY with a chat and its composer all the frame shows, the
composer's window selected; `composer' is the composer."
  (declare (indent 0))
  `(let* ((romacs-agent-log--chats (make-hash-table :test #'equal))
          (session (romacs-agent--make :id "switch-test" :harness 'claude :state 'idle))
          (chat (romacs-agent-log-chat session))
          (composer (romacs-agent-log-composer-buffer chat)))
     (unwind-protect
         (save-window-excursion
           (let ((ignore-window-parameters t)) (delete-other-windows))
           (romacs-agent-log-show session '((display-buffer-same-window)))
           (should (eq (window-buffer) composer))
           ,@body)
       (kill-buffer chat))))

(ert-deftest romacs-agent-compose-switch-the-session-s-own-are-loaded ()
  (should (string-suffix-p "romacs-buffer-switch.el" (symbol-file 'switch-to-buffer 'defun)))
  (should (string-suffix-p "romacs-find-file.el" (symbol-file 'find-file 'defun))))

(ert-deftest romacs-agent-compose-switch-a-buffer-switched-to-shows ()
  ;; C-x b from the composer.
  (romacs-agent-compose-switch-tests--from-composer
    (let ((other (generate-new-buffer "*switched to*")))
      (unwind-protect
          (let ((read-buffer-function (lambda (&rest _) (buffer-name other))))
            (call-interactively #'switch-to-buffer)
            (should (get-buffer-window other))
            (should (get-buffer-window composer)))
        (kill-buffer other)))))

(ert-deftest romacs-agent-compose-switch-a-file-found-shows ()
  ;; C-x C-f from the composer.
  (romacs-agent-compose-switch-tests--from-composer
    (let ((file (make-temp-file "compose-find-" nil ".txt" "found\n")))
      (unwind-protect
          (progn
            (find-file file)
            (should (get-buffer-window (get-file-buffer file)))
            (should (get-buffer-window composer)))
        (when-let* ((buffer (get-file-buffer file))) (kill-buffer buffer))
        (delete-file file)))))

(ert-deftest romacs-agent-compose-switch-a-directory-found-shows ()
  ;; C-x C-f on a directory from the composer: its Dired.
  (romacs-agent-compose-switch-tests--from-composer
    (let ((dir (make-temp-file "compose-dired-" t)))
      (unwind-protect
          (progn
            (find-file dir)
            (should (derived-mode-p 'dired-mode))
            (should (get-buffer-window (current-buffer)))
            (should (get-buffer-window composer)))
        (dolist (buffer (buffer-list))
          (when (equal (buffer-local-value 'default-directory buffer) (file-name-as-directory dir))
            (when (provided-mode-derived-p (buffer-local-value 'major-mode buffer) 'dired-mode)
              (kill-buffer buffer))))
        (delete-directory dir t)))))

(provide 'romacs-agent-compose-switch-tests)
;;; romacs-agent-compose-switch-tests.el ends here
