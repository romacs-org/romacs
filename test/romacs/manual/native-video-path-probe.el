;;; native-video-path-probe.el --- Verify the observed native video path -*- lexical-binding: t -*-

;;; Commentary:

;; Hardware-dependent release acceptance probe.  Run it through
;; scripts/probe-linux-native-video.sh, which guarantees a release Romacs
;; binary and supplies ROMACS_VIDEO_PROBE_FILE.

;;; Code:

(require 'romacs-video)
(require 'seq)

(defvar romacs-video-path-probe--handle nil)
(defvar romacs-video-path-probe--started-at nil)
(defvar romacs-video-path-probe--done nil)

(defun romacs-video-path-probe--positive-integer-p (value)
  "Return non-nil when VALUE is an integer greater than zero."
  (and (integerp value) (> value 0)))

(defun romacs-video-path-probe--zero-p (value)
  "Return non-nil when VALUE is the integer zero."
  (and (integerp value) (= value 0)))

(defun romacs-video-path-probe--coherent-pool-p (pool)
  "Return non-nil when compositor-import POOL reports coherent pressure data."
  (let ((capacity (plist-get pool :capacity))
        (allocated (plist-get pool :allocated))
        (idle (plist-get pool :idle))
        (in-flight (plist-get pool :in-flight))
        (allocations (plist-get pool :allocations))
        (reuses (plist-get pool :reuses))
        (backpressure (plist-get pool :backpressured-acquires))
        (high-water (plist-get pool :in-flight-high-water)))
    (and (romacs-video-path-probe--positive-integer-p capacity)
         (romacs-video-path-probe--positive-integer-p allocated)
         (natnump idle)
         (natnump in-flight)
         (romacs-video-path-probe--positive-integer-p allocations)
         (natnump reuses)
         (romacs-video-path-probe--zero-p backpressure)
         (romacs-video-path-probe--positive-integer-p high-water)
         (= allocated (+ idle in-flight))
         (<= allocated capacity)
         (<= allocated allocations)
         (<= in-flight high-water)
         (<= high-water capacity))))

(defun romacs-video-path-probe--record (text)
  "Record TEXT on the probe's explicit result channel, when configured."
  (when-let ((result-file (getenv "ROMACS_VIDEO_PROBE_RESULT_FILE")))
    (write-region text nil result-file 'append 'silent)))

(defun romacs-video-path-probe--finish (status result &optional snapshot)
  "Exit with STATUS after printing RESULT and optional diagnostic SNAPSHOT."
  (unless romacs-video-path-probe--done
    (setq romacs-video-path-probe--done t)
    (romacs-video-path-probe--record
     (concat
      (format "ROMACS_VIDEO_PROBE_RESULT %s\n" result)
      (when snapshot
        (format "ROMACS_VIDEO_DIAGNOSTICS %S\n" snapshot))))
    (message "ROMACS_VIDEO_PROBE_RESULT %s" result)
    (princ (format "ROMACS_VIDEO_PROBE_RESULT %s\n" result))
    (when snapshot
      (message "ROMACS_VIDEO_DIAGNOSTICS %S" snapshot)
      (princ "ROMACS_VIDEO_DIAGNOSTICS ")
      (prin1 snapshot)
      (princ "\n"))
    (when (and romacs-video-path-probe--handle
               (romacs-video-p romacs-video-path-probe--handle))
      (ignore-errors
        (romacs-video-destroy romacs-video-path-probe--handle)))
    (kill-emacs status)))

(defun romacs-video-path-probe--poll ()
  "Wait for a compositor-visible frame and verify its observed path."
  (condition-case err
      (progn
        ;; Keep the display property in the submitted scene while polling.
        (redisplay t)
        (let* ((snapshot
                (romacs-video-diagnostics
                 romacs-video-path-probe--handle))
               (sessions (plist-get snapshot :sessions))
               (session (car sessions))
               (state (and session (plist-get session :state)))
               (path (and session (plist-get session :frame-path)))
               (import (and path (plist-get path :compositor-import)))
               (presentation (and path (plist-get path :presentation)))
               (format (and session (plist-get session :frame-format)))
               (decoded (and session (plist-get session :decoded-frames)))
               (imported (and session (plist-get session :imported-frames)))
               (backpressured
                (and session (plist-get session :backpressured-frames)))
               (counts (and session (plist-get session :import-counts)))
               (presentation-counts
                (and session (plist-get session :presentation-counts)))
               (borrowed
                (and counts (plist-get counts :borrowed-native-frames)))
               (gpu-blits (and counts (plist-get counts :gpu-blit-frames)))
               (cpu-uploads (and counts (plist-get counts :cpu-upload-frames)))
               (submitted
                (and presentation-counts
                     (plist-get presentation-counts :submitted-frames)))
               (presented
                (and presentation-counts
                     (plist-get presentation-counts :presented-frames)))
               (pools
                (seq-filter
                 (lambda (pool)
                   (eq (plist-get pool :role) 'compositor-import))
                 (plist-get snapshot :surface-pools)))
               (elapsed (- (float-time)
                           romacs-video-path-probe--started-at))
               (timeout (string-to-number
                         (or (getenv "ROMACS_VIDEO_PROBE_TIMEOUT") "15"))))
          (cond
           ((eq state 'failed)
            (romacs-video-path-probe--finish
             1 "FAIL session-failed" snapshot))
           ((and path (not (eq presentation 'wgpu-composited)))
            (romacs-video-path-probe--finish
             1 (format "FAIL presentation=%s" presentation) snapshot))
           ((eq import 'borrowed-native-surface)
            (cond
             ((/= (length sessions) 1)
              (romacs-video-path-probe--finish
               1 (format "FAIL session-count=%s" (length sessions)) snapshot))
             ((not (memq format '(nv12 p010)))
              (romacs-video-path-probe--finish
               1 (format "FAIL direct-frame-format=%s" format) snapshot))
             ((not (and
                    (romacs-video-path-probe--positive-integer-p decoded)
                    (romacs-video-path-probe--positive-integer-p imported)
                    (romacs-video-path-probe--positive-integer-p borrowed)
                    (= borrowed imported)
                    (romacs-video-path-probe--zero-p gpu-blits)
                    (romacs-video-path-probe--zero-p cpu-uploads)
                    (romacs-video-path-probe--zero-p backpressured)
                    (romacs-video-path-probe--positive-integer-p submitted)
                    (romacs-video-path-probe--positive-integer-p presented)
                    (<= presented submitted)))
              (romacs-video-path-probe--finish
               1 "FAIL incoherent-import-or-presentation-counts" snapshot))
             ((/= (length pools) 1)
              (romacs-video-path-probe--finish
               1 (format "FAIL compositor-pool-count=%s" (length pools))
               snapshot))
             ((not (romacs-video-path-probe--coherent-pool-p (car pools)))
              (romacs-video-path-probe--finish
               1 "FAIL incoherent-compositor-pool" snapshot))
             (t
              (romacs-video-path-probe--finish
               0 "PASS direct-yuv-dma-buf-zero-copy" snapshot))))
           ((memq import '(gpu-blit cpu-upload))
            (romacs-video-path-probe--finish
             1 (format "FAIL compositor-import=%s" import) snapshot))
           ((>= elapsed timeout)
            (romacs-video-path-probe--finish
             1 "FAIL timed-out-without-compositor-frame" snapshot)))))
    (error
     (romacs-video-path-probe--finish
      1 (format "FAIL probe-error=%S" err)))))

(defun romacs-video-path-probe--start ()
  "Create one visible playing session and start observing its frame path."
  (let ((file (getenv "ROMACS_VIDEO_PROBE_FILE")))
    (unless (and file (file-readable-p file))
      (romacs-video-path-probe--finish
       1 (format "FAIL unreadable-input=%S" file)))
    (unless (fboundp 'romacs-video-diagnostics)
      (romacs-video-path-probe--finish
       1 "FAIL binary-does-not-expose-video-diagnostics"))
    (switch-to-buffer (get-buffer-create "*Native Video Path Probe*"))
    (erase-buffer)
    (insert "Romacs native video path acceptance probe\n\n")
    (setq romacs-video-path-probe--handle
          (romacs-video-insert file 640 360 0 t))
    (goto-char (point-min))
    (setq romacs-video-path-probe--started-at (float-time))
    (redisplay t)
    (run-at-time 0.1 0.1 #'romacs-video-path-probe--poll)))

(add-hook 'emacs-startup-hook #'romacs-video-path-probe--start)

;;; native-video-path-probe.el ends here
