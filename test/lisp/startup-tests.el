;;; startup-tests.el --- unit tests for startup.el   -*- lexical-binding: t; -*-

;; Copyright (C) 2020-2026 Free Software Foundation, Inc.

;; Author: Philipp Stephani <phst@google.com>

;; This file is part of GNU Emacs.

;; GNU Emacs is free software: you can redistribute it and/or modify
;; it under the terms of the GNU General Public License as published by
;; the Free Software Foundation, either version 3 of the License, or
;; (at your option) any later version.

;; GNU Emacs is distributed in the hope that it will be useful,
;; but WITHOUT ANY WARRANTY; without even the implied warranty of
;; MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
;; GNU General Public License for more details.

;; You should have received a copy of the GNU General Public License
;; along with GNU Emacs.  If not, see <https://www.gnu.org/licenses/>.

;;; Commentary:

;; Unit tests for startup.el.

;;; Code:

(ert-deftest startup-tests/command-switch-alist ()
  (let* ((foo-args ()) (bar-args ())
         (command-switch-alist
          (list (cons "--foo"
                      (lambda (arg)
                        (ert-info ("Processing argument --foo")
                          (push arg foo-args)
                          (should (equal command-line-args-left
                                         '("value" "--bar=value")))
                          (pop command-line-args-left))))
                (cons "--bar=value"
                      (lambda (arg)
                        (ert-info ("Processing argument --bar")
                          (push arg bar-args)
                          (should-not command-line-args-left)))))))
    (command-line-1 '("--foo" "value" "--bar=value"))
    (should (equal foo-args '("--foo")))
    (should (equal bar-args '("--bar=value")))))

(ert-deftest startup-tests/romacs-default-theme-once ()
  (let ((romacs-default-theme 'rum)
        (custom-enabled-themes nil)
        (calls 0))
    (cl-letf (((symbol-function 'load-theme)
               (lambda (theme no-confirm &optional _no-enable)
                 (should (eq theme 'rum))
                 (should no-confirm)
                 (cl-incf calls)
                 (push theme custom-enabled-themes))))
      (romacs--load-default-theme)
      (romacs--load-default-theme)
      (should (= calls 1)))))

(ert-deftest startup-tests/romacs-default-theme-disabled ()
  (let ((romacs-default-theme nil)
        (custom-enabled-themes nil))
    (cl-letf (((symbol-function 'load-theme)
               (lambda (&rest _) (ert-fail "Disabled default loaded"))))
      (romacs--load-default-theme)
      (should-not custom-enabled-themes))))

(ert-deftest startup-tests/romacs-default-theme-preserves-user-theme ()
  (let ((romacs-default-theme 'rum)
        (custom-enabled-themes '(modus-operandi)))
    (cl-letf (((symbol-function 'load-theme)
               (lambda (&rest _) (ert-fail "User theme replaced"))))
      (romacs--load-default-theme)
      (should (equal custom-enabled-themes '(modus-operandi))))))

(ert-deftest startup-tests/rum-independent-of-current-faces ()
  (let ((custom-enabled-themes nil))
    (unwind-protect
        (progn
          (load-theme 'modus-operandi t)
          (load-theme 'rum t t)
          (should (equal custom-enabled-themes '(modus-operandi)))
          (enable-theme 'rum)
          (dolist (face '(fringe window-divider window-divider-first-pixel
                         window-divider-last-pixel))
            (let* ((setting (assq face (mapcar #'cdr (get 'rum 'theme-settings))))
                   (attributes (cadar (nth 2 setting))))
              (should (equal (plist-get attributes :foreground) "#000000"))
              (should (equal (plist-get attributes :background) "#000000")))))
      (disable-theme 'rum)
      (disable-theme 'modus-operandi))))

;;; startup-tests.el ends here
