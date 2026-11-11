;;; romacs-multi-image-perf-test.el --- Multi-image performance test -*- lexical-binding: t -*-

;;; Commentary:
;; Test displaying multiple large images inline to measure performance.

;;; Code:

(defvar multi-image-test-dir (expand-file-name "~/Pictures/"))
(defvar multi-image-count 10)

(defun multi-image-perf-test ()
  "Test displaying multiple large images and measure performance."
  (interactive)
  (let ((buf (get-buffer-create "*Multi-Image Perf Test*"))
        (start-time (current-time))
        (images-loaded 0))
    (switch-to-buffer buf)
    (erase-buffer)
    (insert "*** Romacs Multi-Image Performance Test ***\n\n")
    (insert (format "Loading %d 4K images (3840x2160 each)...\n\n" multi-image-count))

    ;; Record start time
    (let ((load-start (current-time)))
      ;; Insert all images
      (dotimes (i multi-image-count)
        (let* ((img-file (format "%s4k_image_%d.jpg" multi-image-test-dir (1+ i)))
               (img-start (current-time)))
          (insert (format "Image %d: %s\n" (1+ i) img-file))
          (when (file-exists-p img-file)
            (let ((img (create-image img-file nil nil
                                     :max-width 600
                                     :max-height 400)))
              (insert-image img)
              (insert "\n")
              (setq images-loaded (1+ images-loaded))
              (insert (format "  [loaded in %.3f ms]\n\n"
                              (* 1000 (float-time (time-subtract (current-time) img-start)))))))))

      ;; Summary
      (let ((total-time (* 1000 (float-time (time-subtract (current-time) load-start)))))
        (insert "\n=== Performance Summary ===\n")
        (insert (format "Images loaded: %d\n" images-loaded))
        (insert (format "Total load time: %.1f ms\n" total-time))
        (insert (format "Average per image: %.1f ms\n" (/ total-time (max 1 images-loaded))))
        (insert "\nScroll up/down to test rendering performance.\n")
        (insert "Watch for lag or stuttering.\n")
        (insert "\nPress 's' to take screenshot, 'q' to quit.\n")))

    (goto-char (point-min))

    ;; Scroll through buffer to trigger lazy loading of all images
    (insert "\nScrolling to load all images...\n")
    (redisplay t)
    (let ((scroll-start (current-time)))
      (while (not (eobp))
        (scroll-up 1)
        (redisplay t)
        (sit-for 0.05))
      (insert (format "Scroll complete in %.1f ms\n"
                      (* 1000 (float-time (time-subtract (current-time) scroll-start))))))

    (goto-char (point-min))
    (local-set-key "s" (lambda ()
                         (interactive)
                         (let ((f (format "/tmp/romacs-multi-image-%d.png" (emacs-pid))))
                           (romacs-screenshot f)
                           (message "Screenshot: %s" f))))
    (local-set-key "q" #'kill-emacs)
    (message "Multi-image test complete. Press 's' for screenshot, 'q' to quit.")))

;; Run test
(multi-image-perf-test)

;;; romacs-multi-image-perf-test.el ends here
