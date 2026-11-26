;;; romacs-banner-tests.el --- Banners the display moves -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'romacs-banner)

(ert-deftest romacs-banner-still-frames-fill-their-block-in-their-faces ()
  (dolist (pattern '(wave rain sparkle sweep))
    (let* ((faces '(success warning error))
           (still (romacs-banner-still pattern 17 2 faces 5))
           (lines (split-string still "\n")))
      (should (equal (mapcar #'length lines) '(17 17)))
      ;; Every lit cell is in one of the faces; the same epoch, the same frame.
      (dotimes (i (length still))
        (let ((face (get-text-property i 'face still)))
          (should (or (null face) (memq face faces)))))
      (should (equal-including-properties still (romacs-banner-still pattern 17 2 faces 5)))))
  ;; The thread is romacs-working's.
  (should (equal-including-properties (romacs-banner-still 'thread 8 1 '(romacs-working-running) 0 t)
                                      (romacs-working-still 8 'running 'romacs-working-running))))

(ert-deftest romacs-banner-refuses-what-the-display-cannot-draw ()
  (should-error (romacs-banner-still 'wave 0 1 '(success)) :type 'args-out-of-range)
  (should-error (romacs-banner-still 'wave 10 1 nil) :type 'args-out-of-range)
  (should-error (romacs-banner-still 'wave 10 1 '(a b c d e)) :type 'args-out-of-range)
  (should-error (romacs-banner-still 'zigzag 10 1 '(success))))

(ert-deftest romacs-banner-is-registered-only-while-it-moves ()
  (with-temp-buffer
    (insert (romacs-banner-still 'rain 10 1 '(success) 7) "\n")
    (unwind-protect
        (progn
          (romacs-banner-show 'banner-test (current-buffer) (point-min) 'rain 10 1 '(success) nil 7)
          (should-not (romacs-animate--get 'banner-test))
          (romacs-banner-show 'banner-test (current-buffer) (point-min) 'rain 10 1 '(success) t 7)
          (should (romacs-animate--get 'banner-test))
          (should (equal (plist-get (romacs-banner--element 'banner-test) :kind) '(banner rain 7 (success))))
          ;; Shown again moving: the same registration, so its motion goes on.
          (let ((key (romacs-animate--slot (romacs-animate--get 'banner-test) :key)))
            (romacs-banner-show 'banner-test (current-buffer) (point-min) 'rain 10 1 '(success warning) t 7)
            (should (eq key (romacs-animate--slot (romacs-animate--get 'banner-test) :key)))
            (should (equal (plist-get (romacs-banner--element 'banner-test) :kind)
                           '(banner rain 7 (success warning)))))
          ;; The thread pattern hands over to romacs-working.
          (romacs-banner-show 'banner-test (current-buffer) (point-min) 'thread 10 1 '(romacs-working-running) t)
          (should-not (romacs-banner--element 'banner-test))
          (should (romacs-working--element 'banner-test)))
      (romacs-banner-stop 'banner-test))
    (should-not (romacs-animate--get 'banner-test))
    (should-not (romacs-working--element 'banner-test))))

(ert-deftest romacs-banner-stops-with-its-buffer ()
  (let ((buffer (generate-new-buffer " banner")))
    (with-current-buffer buffer (insert (romacs-banner-still 'wave 6 1 '(success)) "\n"))
    (romacs-banner-show 'banner-kill buffer 1 'wave 6 1 '(success) t)
    (should (romacs-animate--get 'banner-kill))
    (kill-buffer buffer)
    (should-not (romacs-animate--get 'banner-kill))
    (should-not (romacs-banner--element 'banner-kill))))

;;; romacs-banner-tests.el ends here
