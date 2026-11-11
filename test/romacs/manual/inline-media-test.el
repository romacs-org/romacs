;;; inline-media-test.el --- Test inline image and video in one buffer -*- lexical-binding: t -*-

;; This test creates a buffer containing both inline media types:
;; - Image (via standard Emacs create-image/insert-image)
;; - Video (via romacs-video-insert)

;;; Commentary:
;; Run with: ./test/romacs/manual/run-inline-media-test.sh
;; Or manually: DISPLAY=:0 ./target/release/romacs -Q -l test/romacs/manual/inline-media-test.el

;;; Code:

(require 'romacs-video nil t)

(defvar inline-media-test-image-path
  (or (let ((home-pic (expand-file-name "~/Pictures/559-4K.jpg")))
        (and (file-exists-p home-pic) home-pic))
      (let ((test-pic (expand-file-name "test/gnu/test/data/image/black.jpg")))
        (and (file-exists-p test-pic) test-pic))
      nil)
  "Path to test image, or nil if not found.")

(defvar inline-media-test-video-path
  (or (let ((home-vid (expand-file-name "~/Videos/4k_f1.mp4")))
        (and (file-exists-p home-vid) home-vid))
      (let ((test-vid (expand-file-name "test/data/video/test.mp4")))
        (and (file-exists-p test-vid) test-vid))
      nil)
  "Path to first test video, or nil if not found.")

(defvar inline-media-test-video-path-2
  (or (let ((home-vid (expand-file-name "~/Videos/4k_test.mp4")))
        (and (file-exists-p home-vid) home-vid))
      (let ((test-vid (expand-file-name "test/data/video/test.mp4")))
        (and (file-exists-p test-vid) test-vid))
      nil)
  "Path to second test video, or nil if not found.")



(defun inline-media-test-log (fmt &rest args)
  "Log FMT with ARGS to stderr."
  (princ (concat (apply #'format fmt args) "\n") #'external-debugging-output))


(defun inline-media-test-run ()
  "Run the inline media test."
  (interactive)

  ;; Resize frame large enough for all media side-by-side
  (set-frame-size (selected-frame) 160 70)  ; columns x lines
  (set-frame-position (selected-frame) 50 50)

  (switch-to-buffer (get-buffer-create "*Inline Media Test*"))
  (erase-buffer)
  (setq buffer-read-only nil)

  (insert "=== Romacs Inline Media Test ===\n")
  (insert "This buffer contains inline image and video.\n\n")

  (let ((media-width 320)
        (media-height 180)  ; 16:9 aspect
        (success-count 0)
        (total-count 2))

    ;; === Section 2: Inline Video (display property with loop+autoplay) ===
    (insert "--- 2. Inline Video (display prop, loop+autoplay) ---\n")
    (if (and inline-media-test-video-path inline-media-test-video-path-2)
        (condition-case err
            (progn
              ;; Load videos and insert via display properties directly
              ;; (tests :loop-count and :autoplay pipeline end-to-end)
              (let* ((uri-1 (concat "file://" (expand-file-name
                                               inline-media-test-video-path)))
                     (uri-2 (concat "file://" (expand-file-name
                                               inline-media-test-video-path-2)))
                     (video-id-1 (romacs-video-load uri-1))
                     (video-id-2 (romacs-video-load uri-2)))
                (when video-id-1
                  (let ((start (point)))
                    (insert " ")
                    (put-text-property start (point) 'display
                      `(video :id ,video-id-1
                              :width ,media-width :height ,media-height
                              :loop-count -1 :autoplay t))
                    (put-text-property start (point)
                                       'romacs-video-id video-id-1))
                  (romacs-video-play video-id-1))
                (when video-id-2
                  (let ((start (point)))
                    (insert " ")
                    (put-text-property start (point) 'display
                      `(video :id ,video-id-2
                              :width ,media-width :height ,media-height
                              :loop-count 3 :autoplay t))
                    (put-text-property start (point)
                                       'romacs-video-id video-id-2))
                  (romacs-video-play video-id-2))
                (cond
                 ((and video-id-1 video-id-2)
                  (insert "\n")
                  (insert (format
                    "Video 1: %s (ID %d, loop=-1) Video 2: %s (ID %d, loop=3) %dx%d\n"
                    (file-name-nondirectory inline-media-test-video-path)
                    video-id-1
                    (file-name-nondirectory inline-media-test-video-path-2)
                    video-id-2
                    media-width media-height))
                  (insert "Both auto-playing via display property.\n")
                  (setq success-count (1+ success-count))
                  (inline-media-test-log
                    "Video: OK (id1=%d loop=-1, id2=%d loop=3)"
                    video-id-1 video-id-2))
                 (t
                  (insert "\n[Video creation failed]\n")
                  (inline-media-test-log
                    "Video: FAILED (one or both returned nil)")))))
          (error
           (insert (format "[Video error: %S]\n" err))
           (inline-media-test-log "Video: ERROR %S" err)))
      (insert "[No test video found - skipping]\n")
      (insert "Need both ~/Videos/4k_f1.mp4 and ~/Videos/4k_test.mp4\n")
      (inline-media-test-log "Video: SKIPPED (no file)"))
    (insert "\n")

    ;; === Section 3: Inline Image ===
    (insert "--- 3. Inline Image ---\n")
    (if inline-media-test-image-path
        (condition-case err
            (let ((img (create-image inline-media-test-image-path nil nil
                                     :max-width media-width
                                     :max-height media-height)))
              (if img
                  (progn
                    (insert-image img "[IMAGE]")
                    (insert "\n")
                    (insert (format "Image: %s (%dx%d max)\n"
                                    (file-name-nondirectory inline-media-test-image-path)
                                    media-width media-height))
                    (setq success-count (1+ success-count))
                    (inline-media-test-log "Image: OK"))
                (insert "[Image creation failed]\n")
                (inline-media-test-log "Image: FAILED (create-image returned nil)")))
          (error
           (insert (format "[Image error: %S]\n" err))
           (inline-media-test-log "Image: ERROR %S" err)))
      (insert "[No test image found - skipping]\n")
      (insert "Set inline-media-test-image-path or place image at ~/Pictures/559-4K.jpg\n")
      (inline-media-test-log "Image: SKIPPED (no file)"))
    (insert "\n")

    ;; === Summary ===
    (insert "--- Summary ---\n")
    (insert (format "Media types loaded: %d/%d\n" success-count total-count))
    (if (= success-count total-count)
        (insert "All inline media types working!\n")
      (insert "Some media types failed or were skipped.\n"))

    (inline-media-test-log "=== Result: %d/%d media types loaded ===" success-count total-count))

  (goto-char (point-min))
  (setq buffer-read-only t)
  (redisplay t)

  ;; Keep running for manual inspection (shell script handles screenshot)
  (inline-media-test-log "Test buffer ready. Keeping Emacs open for 30 seconds...")
  (run-at-time 30 nil
               (lambda ()
                 (inline-media-test-log "Test timeout - exiting")
                 (kill-emacs 0))))

;; Auto-run when loaded
(add-hook 'emacs-startup-hook #'inline-media-test-run)

(provide 'inline-media-test)
;;; inline-media-test.el ends here
