;;; mail-send-test.el --- Background submission lifecycle -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

(defvar mail-test-callback nil)
(defvar mail-test-submissions 0)

(defun mail-test-runner (argv input directory timeout callback)
  (setq mail-test-callback callback mail-test-submissions (1+ mail-test-submissions))
  41)

(defun mail-test-draft ()
  (romacs-mail-compose '(:name "fixture" :from "user@example.invalid" :send ("fixture-send"))
                       "recipient@example.invalid" "Fixture")
  (insert "Body λ\n")
  (current-buffer))

(defun mail-test-submit ()
  (let ((original (symbol-function 'read-string))
        (romacs-mail-runner 'mail-test-runner))
    (unwind-protect
        (progn (fset 'read-string (lambda (&rest ignored) "yes")) (romacs-mail-send))
      (fset 'read-string original))))

(defun mail-test-echo ()
  (with-current-buffer (window-buffer (minibuffer-window)) (buffer-string)))

(romacs-deftest mail-send-hides-then-deletes-on-success ()
  (let ((parent (current-buffer)) (draft nil) (mail-test-submissions 0))
    (unwind-protect
        (progn
          (setq draft (mail-test-draft))
          (mail-test-submit)
          (romacs-should (= mail-test-submissions 1))
          (romacs-should (eq (current-buffer) parent))
          (romacs-should-not (get-buffer-window draft))
          (romacs-should (buffer-live-p draft))
          (with-current-buffer draft (romacs-should (eq romacs-mail-send-state 'sending)))
          (romacs-should (funcall mail-test-callback '(0 "" "" nil)))
          (romacs-should-not (buffer-live-p draft))
          (romacs-should (eq (current-buffer) parent))
          (romacs-should (equal (mail-test-echo) "Email to recipient@example.invalid sent successfully"))
          (funcall mail-test-callback '(0 "" "" nil))
          (romacs-should (= mail-test-submissions 1)))
      (when (buffer-live-p draft) (kill-buffer draft))
      (switch-to-buffer parent))))

(romacs-deftest mail-send-failure-saves-one-hidden-draft ()
  (let ((parent (current-buffer)) (draft nil) (mail-test-submissions 0)
        (romacs-mail-draft-directory (expand-file-name "~/send-drafts/")))
    (unwind-protect
        (progn
          (setq draft (mail-test-draft))
          (let ((text (buffer-string)))
            (mail-test-submit)
            (romacs-should (funcall mail-test-callback '(1 "" "Authentication failed\n" nil)))
            (romacs-should (eq (current-buffer) parent))
            (romacs-should-not (get-buffer-window draft))
            (with-current-buffer draft
              (romacs-should (equal (buffer-string) text))
              (romacs-should-not buffer-read-only)
              (romacs-should-not romacs-mail-send-state)
              (romacs-should (file-attributes romacs-mail-draft-file))
              (romacs-should (equal (with-temp-buffer (insert-file-contents (with-current-buffer draft romacs-mail-draft-file)) (buffer-string)) text)))
            (let ((files (directory-files romacs-mail-draft-directory nil "\\.draft$")))
              (funcall mail-test-callback '(1 "" "Authentication failed\n" nil))
              (romacs-should (equal files (directory-files romacs-mail-draft-directory nil "\\.draft$")))))
          (romacs-should
           (equal (mail-test-echo)
                  (concat "Email to recipient@example.invalid failed: Authentication failed; saved as draft: "
                          (with-current-buffer draft romacs-mail-draft-file)))))
      (when (buffer-live-p draft) (kill-buffer draft))
      (switch-to-buffer parent))))

(romacs-deftest mail-send-accepted-is-not-a-failed-draft ()
  (let ((parent (current-buffer)) (draft nil))
    (unwind-protect
        (progn
          (setq draft (mail-test-draft))
          (mail-test-submit)
          (funcall mail-test-callback '(75 "" "Sent verification failed" nil))
          (romacs-should-not (buffer-live-p draft))
          (romacs-should (string-match "sent.*Sent" (mail-test-echo)))
          (romacs-should (eq (current-buffer) parent)))
      (when (buffer-live-p draft) (kill-buffer draft))
      (switch-to-buffer parent))))

(romacs-deftest mail-send-outcomes-distinguish-transport-evidence ()
  (romacs-should (eq (romacs-mail-send-outcome '(0 "" "" nil)) 'sent))
  (romacs-should (eq (romacs-mail-send-outcome '(75 "" "" nil)) 'accepted))
  (romacs-should (eq (romacs-mail-send-outcome '(1 "" "refused" nil)) 'failed))
  (romacs-should (eq (romacs-mail-send-outcome '(76 "" "disconnect" nil)) 'unknown))
  (romacs-should (eq (romacs-mail-send-outcome '(nil "" "" "CLI job timed out")) 'unknown)))

(romacs-deftest mail-send-does-not-kill-a-repurposed-draft ()
  (let ((parent (current-buffer)) (draft nil))
    (unwind-protect
        (progn
          (setq draft (mail-test-draft))
          (mail-test-submit)
          (with-current-buffer draft
            (let ((inhibit-read-only t)) (erase-buffer) (insert "New user document"))
            (setq buffer-file-name "/new-user-document"))
          (funcall mail-test-callback '(0 "" "" nil))
          (romacs-should (buffer-live-p draft))
          (with-current-buffer draft
            (romacs-should (equal (buffer-string) "New user document"))
            (romacs-should (equal buffer-file-name "/new-user-document"))))
      (when (buffer-live-p draft) (kill-buffer draft))
      (switch-to-buffer parent))))

(romacs-deftest mail-send-killed-failure-is-saved-without-reopening ()
  (let ((parent (current-buffer)) (draft nil)
        (romacs-mail-draft-directory (expand-file-name "~/killed-drafts/")))
    (unwind-protect
        (progn
          (setq draft (mail-test-draft))
          (mail-test-submit)
          (kill-buffer draft)
          (funcall mail-test-callback '(1 "" "Refused" nil))
          (romacs-should (eq (current-buffer) parent))
          (romacs-should-not (buffer-live-p draft))
          (romacs-should (= (length (directory-files romacs-mail-draft-directory nil "\\.draft$")) 1)))
      (when (buffer-live-p draft) (kill-buffer draft))
      (switch-to-buffer parent))))

(romacs-deftest mail-send-unknown-saves-without-automatic-retry ()
  (let ((parent (current-buffer)) (draft nil) (mail-test-submissions 0)
        (romacs-mail-draft-directory (expand-file-name "~/unknown-drafts/")))
    (unwind-protect
        (progn
          (setq draft (mail-test-draft))
          (mail-test-submit)
          (funcall mail-test-callback '(76 "" "Disconnected after submission" nil))
          (with-current-buffer draft
            (romacs-should (eq romacs-mail-send-state 'uncertain))
            (romacs-should (file-attributes romacs-mail-draft-file)))
          (romacs-should (= mail-test-submissions 1))
          (romacs-should (eq (current-buffer) parent))
          (with-current-buffer draft (mail-test-submit))
          (romacs-should (= mail-test-submissions 2)))
      (when (buffer-live-p draft) (kill-buffer draft))
      (switch-to-buffer parent))))

(provide 'mail-send-test)
