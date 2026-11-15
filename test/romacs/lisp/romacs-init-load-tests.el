;;; romacs-init-load-tests.el --- The user's init, and what stops it -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-init-load)

;; What `romacs-startup-progress' keeps: bound here around each failure.
(defvar romacs-progress-state)

(defmacro romacs-init-load-tests--with (text &rest body)
  "BODY with `init' an init file holding TEXT."
  (declare (indent 1))
  `(let ((init (make-temp-file "romacs-init-" nil ".el" ,text)))
     (unwind-protect (progn ,@body) (delete-file init))))

(ert-deftest romacs-init-load-an-init-that-loads-says-nothing ()
  (romacs-init-load-tests--with "(setq romacs-init-load-tests--a 1)\n"
    (should-not (romacs-init-load init))
    (should-not romacs-init--backtrace)))

(ert-deftest romacs-init-load-a-void-function-names-its-form-and-keeps-its-backtrace ()
  ;; Worded as the loader words an error (`FILE: line N form M: FORM:
  ;; ERROR'), its backtrace taken where it happened, in the init's form.
  (romacs-init-load-tests--with
      "(setq romacs-init-load-tests--a 1)\n\n(defun romacs-init-load-tests--broken () (romacs-init-load-tests--no-such 1))\n(romacs-init-load-tests--broken)\n(setq romacs-init-load-tests--b 2)\n"
    (let ((romacs-init-load-tests--b nil))
      (should (equal (romacs-init-load init)
                     (format "%s: line 4 form 3: (romacs-init-load-tests--broken): (void-function romacs-init-load-tests--no-such)"
                             init)))
      ;; It stopped there.
      (should-not romacs-init-load-tests--b))
    (should (string-match-p "romacs-init-load-tests--broken()" romacs-init--backtrace))
    (when (fboundp 'romacs--loads-in-progress)
      (should (string-prefix-p (format "In %s, line 4 (form 3)" init) romacs-init--backtrace)))))

(ert-deftest romacs-init-load-a-syntax-error-and-a-missing-package-stop-it ()
  (romacs-init-load-tests--with "(setq romacs-init-load-tests--a 1)\n(setq romacs-init-load-tests--b (\n"
    (let ((said (romacs-init-load init)))
      (should (string-prefix-p (format "%s: line " init) said))
      (should (string-match-p "end-of-file" said))))
  (romacs-init-load-tests--with "(require 'romacs-init-load-tests--no-such-package)\n"
    (let ((said (romacs-init-load init)))
      (should (string-match-p "line 1 form 1: (require 'romacs-init-load-tests--no-such-package): (file-missing" said))
      (should (string-match-p "require(romacs-init-load-tests--no-such-package)" romacs-init--backtrace)))))

(ert-deftest romacs-init-load-a-quit-stops-it-as-an-error-does ()
  ;; C-g in a form that would not end: the init stops there, said.
  (romacs-init-load-tests--with "(setq romacs-init-load-tests--a 1)\n(signal 'quit nil)\n"
    (should (equal (romacs-init-load init) (format "%s: line 2 form 2: (signal 'quit nil): (quit)" init)))
    (should romacs-init--backtrace)))

(ert-deftest romacs-init-load-a-failure-is-a-warning-with-its-backtrace-a-button-away ()
  ;; GNU's startup's warning, in *Warnings*; the backtrace replaces its
  ;; button there; the launch screen's init stage failed.
  (romacs-init-load-tests--with "(romacs-init-load-tests--no-such 1)\n"
    (let ((romacs-progress-state nil) (said (romacs-init-load init)))
      (unwind-protect
          (progn
            (romacs-init-failed said)
            (with-current-buffer "*Warnings*"
              (should (string-match-p (regexp-quote said) (buffer-string)))
              (goto-char (point-min))
              (should (search-forward "Show the backtrace" nil t))
              (let ((button (button-at (1- (point)))))
                (should button)
                (button-activate button))
              (should-not (string-match-p "Show the backtrace" (buffer-string)))
              (should (string-match-p "romacs-init-load-tests--no-such" (buffer-string))))
            (let ((init-state (alist-get 'init romacs-progress-state)))
              (should (eq (plist-get init-state :status) 'failed))
              (should (equal (plist-get init-state :error) said))))
        (kill-buffer "*Warnings*")))))

;;; romacs-init-load-tests.el ends here
