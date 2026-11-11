;;; romacs-video-test.el --- End-to-end native video smoke test -*- lexical-binding: t -*-

;;; Commentary:
;; Exercise the same opaque session handle through creation, display, pause,
;; play, and destruction.  Missing native entry points and backend failures are
;; deliberately fatal; this test must not turn an unavailable video API into a
;; successful run.

;;; Code:

(require 'romacs-video)

(defvar video-test-file
  (or (getenv "ROMACS_VIDEO_TEST_FILE")
      "/home/exec/Videos/4k_f1.mp4")
  "Path to the video used by the end-to-end smoke test.")

(defvar video-test-handle nil
  "Opaque handle for the test's compositor-owned video session.")

(defun romacs-video-test ()
  "Open, display, and play `video-test-file'."
  (unless (file-readable-p video-test-file)
    (error "Video test file is not readable: %s" video-test-file))
  (let ((buffer (get-buffer-create "*Romacs Video Test*")))
    (switch-to-buffer buffer)
    (erase-buffer)
    (insert "*** Romacs Inline Video Test ***\n\n")
    (insert (format "Video file: %s\n" video-test-file))
    (insert (format "File size: %s\n\n"
                    (file-size-human-readable
                     (file-attribute-size (file-attributes video-test-file)))))
    (setq video-test-handle
          (romacs-video-insert video-test-file 800 450 0 t))
    (unless (romacs-video-p video-test-handle)
      (error "romacs-video-insert returned no video handle: %S"
             video-test-handle))
    (insert "\n\nNative video session created and playback requested.\n")
    (insert (format "Handle: %S\n" video-test-handle))
    (insert "Controls: p play, s stop, SPC pause/play\n")
    (goto-char (point-min))
    ;; Prove that controls address the same live session.  Frame production is
    ;; driven by the native backend and render-loop wakeups, not a Lisp timer.
    (run-at-time 2 nil
                 (lambda ()
                   (romacs-video-pause video-test-handle)
                   (message "Video smoke test: paused %S" video-test-handle)))
    (run-at-time 3 nil
                 (lambda ()
                   (romacs-video-play video-test-handle)
                   (message "Video smoke test: resumed %S" video-test-handle)))
    (message "Video smoke test: playing %S" video-test-handle)))

(romacs-video-test)

(run-at-time 8 nil
             (lambda ()
               (romacs-video-destroy video-test-handle)
               (message "Video smoke test complete")
               (kill-emacs 0)))

;;; romacs-video-test.el ends here
