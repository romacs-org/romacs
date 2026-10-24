;;; video-auto-mode-test.el --- video files must not pollute `auto-mode-alist' -*- lexical-binding: t -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

;; This file is part of Romacs.

;;; Commentary:

;; GNU maps none of the video extensions Romacs plays, so the entry that
;; selects `romacs-video-mode' must not live in `auto-mode-alist': that
;; variable is GNU's and its contents are observable, so a Romacs-only
;; extension in it reports a value GNU never produces (the parity suite pins
;; the length).  Romacs therefore keeps its own registry
;; (`romacs-video-file-name-regexp') and consults it from `find-file-hook'.
;;
;; This test pins both halves: `auto-mode-alist' stays exactly as GNU leaves
;; it, and the Romacs-only registry still selects the mode.
;;
;; Unlike its siblings in this directory this needs no display -- it is pure
;; mode-selection logic, so it runs in batch and asserts:
;;
;;   ./target/release/romacs -Q --batch -l test/romacs/video-auto-mode-test.el
;;
;; Exit status is 0 when every check passes, 1 otherwise.

;;; Code:

(require 'cl-lib)
(require 'seq)

(defvar video-auto-mode-test--checks 0)
(defvar video-auto-mode-test--failures nil)

(defun video-auto-mode-test--check (label actual expected)
  "Record that LABEL's ACTUAL equals EXPECTED."
  (setq video-auto-mode-test--checks (1+ video-auto-mode-test--checks))
  (unless (equal actual expected)
    (push (format "%s\n    expected: %S\n    actual:   %S" label expected actual)
          video-auto-mode-test--failures)))

;;; The GNU-visible variable must be untouched.

;; GNU 31.1's `auto-mode-alist' has 268 entries; this is the number the parity
;; suite's div_f9 pins, and the whole point of the registry is that Romacs
;; keeps producing it.
(video-auto-mode-test--check "auto-mode-alist length"
                             (length auto-mode-alist) 268)

(video-auto-mode-test--check
 "no video extension in auto-mode-alist"
 (seq-filter (lambda (entry)
               (string-match-p "\\.\\(?:avi\\|m4v\\|mkv\\|mov\\|mp4\\|mpeg\\|mpg\\|ogv\\|webm\\)\\'"
                               (car entry)))
             auto-mode-alist)
 nil)

;;; The Romacs-only registry and its hook.

(video-auto-mode-test--check "video file not loaded at startup"
                             (featurep 'romacs-video) nil)

(video-auto-mode-test--check
 "selection function is autoloaded, not loaded"
 (autoloadp (symbol-function 'romacs-video--select-visited-video)) t)

;; Loading the file must leave exactly one hook entry, however many times it
;; happens: this is what `add-hook' gives over a raw `advice-add'.
(require 'romacs-video)
(load "romacs-video" nil t)
(video-auto-mode-test--check
 "hook installed exactly once even after a second load"
 (seq-count (lambda (entry) (eq entry #'romacs-video--select-visited-video))
            find-file-hook)
 1)

(video-auto-mode-test--check "registry matches a video name"
                             (string-match-p romacs-video-file-name-regexp
                                             "/x/clip.mp4")
                             7)

;;; The selection decision.  The real mode body needs a decodable video and a
;;; display, so stub it and check only which buffers the hook claims.

(let ((selected nil))
  (cl-letf (((symbol-function 'romacs-video-mode)
             (lambda (&rest _) (setq selected 'video-mode))))

    (with-temp-buffer
      (setq buffer-file-name "/tmp/video-auto-mode-test.mp4")
      (fundamental-mode)
      (setq selected nil)
      (romacs-video--select-visited-video)
      (video-auto-mode-test--check "video file left in fundamental mode is claimed"
                                   selected 'video-mode))

    (with-temp-buffer
      (setq buffer-file-name "/tmp/video-auto-mode-test.txt")
      (fundamental-mode)
      (setq selected nil)
      (romacs-video--select-visited-video)
      (video-auto-mode-test--check "non-video file is not claimed" selected nil))

    ;; Every explicit rule GNU runs keeps precedence, because only a buffer
    ;; still in `fundamental-mode' is claimed.
    (with-temp-buffer
      (setq buffer-file-name "/tmp/video-auto-mode-test.mp4")
      (text-mode)
      (setq selected nil)
      (romacs-video--select-visited-video)
      (video-auto-mode-test--check "video file with an explicit mode is not claimed"
                                   selected nil))

    (with-temp-buffer
      (fundamental-mode)
      (setq selected nil)
      (romacs-video--select-visited-video)
      (video-auto-mode-test--check "a buffer with no file is not claimed"
                                   selected nil))))

;;; Report.

(if video-auto-mode-test--failures
    (progn
      (princ (format "video-auto-mode-test: %d of %d checks FAILED\n"
                     (length video-auto-mode-test--failures)
                     video-auto-mode-test--checks))
      (dolist (failure (nreverse video-auto-mode-test--failures))
        (princ (format "  - %s\n" failure)))
      (kill-emacs 1))
  (princ (format "video-auto-mode-test: all %d checks passed\n"
                 video-auto-mode-test--checks))
  (kill-emacs 0))

;;; video-auto-mode-test.el ends here
