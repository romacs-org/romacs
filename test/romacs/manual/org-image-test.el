;;; org-image-test.el --- Test org-mode inline image preview -*- lexical-binding: t -*-

;; Verifies that org-toggle-inline-images works correctly in Romacs.
;; This was broken because the romacs image validator rejected :width nil
;; and was missing standard image keywords. (Issue #40)

;;; Code:

(defvar org-image-test--log-file "/tmp/romacs-org-image-test.log")
(defvar org-image-test--screenshot-file "/tmp/romacs-org-image-test.png")

(defun org-image-test--log (fmt &rest args)
  "Append a line to the log file."
  (let ((msg (apply #'format fmt args)))
    (append-to-file (concat msg "\n") nil org-image-test--log-file)
    (message "%s" msg)))

(defun org-image-test--run ()
  "Run the org inline image test."
  ;; Clear log
  (when (file-exists-p org-image-test--log-file)
    (delete-file org-image-test--log-file))

  (org-image-test--log "=== Org Image Preview Test ===")

  ;; Find a test image
  (let ((test-image (or (let ((gnu (expand-file-name "test/gnu/test/data/image/black.jpg"
                                                     (or (getenv "EMACS_SOURCE_DIR")
                                                         (file-name-directory
                                                          (or load-file-name buffer-file-name
                                                              default-directory))))))
                          (and (file-exists-p gnu) gnu))
                        ;; Download a small test image
                        (let ((tmp "/tmp/romacs-org-test-image.png"))
                          (unless (file-exists-p tmp)
                            (url-copy-file "https://www.gnu.org/graphics/heckert_gnu.small.png" tmp t))
                          (and (file-exists-p tmp) tmp)))))

    (unless test-image
      (org-image-test--log "FAIL: No test image available")
      (sit-for 3)
      (kill-emacs 1))

    (org-image-test--log "Test image: %s" test-image)

    ;; Create org buffer with image link
    (let ((org-buf (get-buffer-create "*Org Image Test*")))
      (switch-to-buffer org-buf)
      (org-mode)
      (erase-buffer)
      (insert "* Org Image Preview Test\n\n")
      (insert "This tests that org-toggle-inline-images works in Romacs.\n\n")
      (insert "** PNG Image\n\n")
      (insert (format "[[file:%s]]\n\n" test-image))
      (insert "** End of test\n")

      ;; Test 1: image-type-available-p
      (org-image-test--log "image-type-available-p png: %s" (image-type-available-p 'png))
      (org-image-test--log "image-type-available-p jpeg: %s" (image-type-available-p 'jpeg))

      ;; Test 2: create-image with keywords org-mode uses
      (condition-case err
          (progn
            (create-image test-image nil nil :width nil :max-width 560 :scale 1)
            (org-image-test--log "PASS: create-image with :width nil"))
        (error (org-image-test--log "FAIL: create-image with :width nil: %S" err)))

      (condition-case err
          (progn
            (create-image test-image nil nil :ascent 'center)
            (org-image-test--log "PASS: create-image with :ascent center"))
        (error (org-image-test--log "FAIL: create-image with :ascent center: %S" err)))

      ;; Test 3: org-toggle-inline-images
      (condition-case err
          (progn
            (org-toggle-inline-images)
            (org-image-test--log "PASS: org-toggle-inline-images succeeded")
            ;; Verify overlay was created
            (goto-char (point-min))
            (let ((found nil))
              (while (not (eobp))
                (dolist (ov (overlays-at (point)))
                  (let ((disp (overlay-get ov 'display)))
                    (when (and disp (eq (car-safe disp) 'image))
                      (setq found t)
                      (org-image-test--log "PASS: Found image overlay: %S"
                                           (seq-take disp 8)))))
                (forward-char 1))
              (unless found
                (org-image-test--log "FAIL: No image display overlays found"))))
        (error (org-image-test--log "FAIL: org-toggle-inline-images: %S" err)))

      (org-image-test--log "=== Test Complete ===")

      ;; Wait for rendering, then take screenshot via xdotool+import
      (sit-for 3)
      (kill-emacs))))

;; Auto-run after init
(add-hook 'emacs-startup-hook #'org-image-test--run)

;;; org-image-test.el ends here
