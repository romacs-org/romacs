;;; romacs-working-tests.el --- The working thread -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-working)

(ert-deftest romacs-working-says-its-work-to-the-display ()
  (with-temp-buffer
    (insert (romacs-working-still 12 'running) " running\n")
    (unwind-protect
        (progn
          (romacs-working-show 'test (current-buffer) (point-min) 12 'running)
          (should (equal (plist-get (romacs-working--element 'test) :kind)
                         '(working running 0 romacs-working-running)))
          (should (equal (plist-get (romacs-working--element 'test) :cols) 12))
          (should-not (romacs-animate-paused-p 'test))
          ;; At rest it holds still; waiting it moves again.
          (romacs-working-set 'test 'idle)
          (should (romacs-animate-paused-p 'test))
          (romacs-working-set 'test 'waiting)
          (should-not (romacs-animate-paused-p 'test))
          (should (eq (nth 1 (plist-get (romacs-working--element 'test) :kind)) 'waiting)))
      (romacs-working-stop 'test))
    (should-not (romacs-working--element 'test))))

(ert-deftest romacs-working-follows-its-callers-marker ()
  (with-temp-buffer
    (insert "before\n")
    (let ((start (copy-marker (point) t)))
      (insert (romacs-working-still 4 'idle))
      (unwind-protect
          (progn
            (romacs-working-show 'moving (current-buffer) start 4)
            (goto-char (point-min))
            (insert "more\n")
            (should (= (plist-get (romacs-working--element 'moving) :start) (marker-position start))))
        (romacs-working-stop 'moving))
      ;; The caller's marker is the caller's: still there.
      (should (marker-position start)))))

(ert-deftest romacs-working-still-frames-are-as-wide-as-asked ()
  (dolist (work '(idle running waiting failed))
    (should (= (length (romacs-working-still 9 work)) 9))))

(provide 'romacs-working-tests)
;;; romacs-working-tests.el ends here
