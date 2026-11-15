;;; romacs-mode-line-tests.el --- The status bar's buffer path  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `romacs-mode-line--path' names a file by `romacs-mode-line-buffer-style'.
;; A project rooted at the home directory has the abbreviated root `~/',
;; which has no parent: the path once signalled (wrong-type-argument
;; stringp nil) there, in every redisplay of such a buffer (#206).

;;; Code:

(require 'ert)
(require 'romacs-mode-line)

(ert-deftest romacs-mode-line-path-under-a-home-rooted-project ()
  (dolist (style '(file-name file-name-with-project relative-to-project relative-from-project
                   truncate-upto-project truncate-from-project truncate-with-project
                   truncate-except-project truncate-upto-root truncate-all truncate-nil))
    (should (stringp (romacs-mode-line--path "~/src/x.el" "~/" style))))
  (should (equal (romacs-mode-line--path "~/src/x.el" "~/" 'truncate-with-project) "~/s/x.el"))
  (should (equal (romacs-mode-line--path "~/src/x.el" "~/" 'relative-to-project) "src/x.el")))

(ert-deftest romacs-mode-line-path-under-a-nested-project ()
  (should (equal (romacs-mode-line--path "/tmp/a/proj/src/x.el" "/tmp/a/proj/" 'truncate-with-project) "proj/s/x.el"))
  (should (equal (romacs-mode-line--path "/tmp/a/proj/src/x.el" "/tmp/a/proj/" 'truncate-upto-project) "/t/a/proj/src/x.el")))

;; A segment's computation runs from the hooks that make it stale, as
;; `after-revert-hook' and `after-save-hook'.  One that signalled took the
;; revert with it, and with it `auto-revert-buffers''s timer (#207).
(romacs-mode-line-define-segment test-signalling
  "A segment whose computation signals."
  :compute (lambda () (error "Segment failure"))
  :hooks (after-revert-hook))

(ert-deftest romacs-mode-line-a-failing-segment-leaves-its-hook-alone ()
  (let ((file (make-temp-file "romacs-mode-line" nil ".txt" "one\n")))
    (unwind-protect
        (with-current-buffer (find-file-noselect file)
          (unwind-protect
              (progn
                (should-not (condition-case nil (progn (romacs-mode-line-refresh-test-signalling) nil) (error t)))
                (should-not romacs-mode-line-test-signalling)
                (with-temp-file file (insert "two\n"))
                (should-not (condition-case err (progn (revert-buffer t t) nil) (error err)))
                (should (equal (buffer-string) "two\n")))
            (set-buffer-modified-p nil)
            (kill-buffer)))
      (delete-file file))))

;;; romacs-mode-line-tests.el ends here
