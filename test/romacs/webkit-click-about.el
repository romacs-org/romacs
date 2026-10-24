;;; webkit-click-about.el --- Test clicking "About" on Google -*- lexical-binding: t -*-

;;; Code:

(require 'romacs-webkit nil t)

(defvar test-view-id nil)
(defvar test-width nil)  ;; Will be auto-calculated
(defvar test-height nil)

(defun test-log (fmt &rest args)
  "Log to stderr."
  (princ (concat (apply #'format fmt args) "\n") #'external-debugging-output))

(defun test-click-about ()
  "Click the About link on Google homepage."
  (test-log "=== Test: Click 'About' on Google ===")

  ;; On Google's homepage, "About" is typically in the top-left area
  ;; It's usually around x=30, y=20 from top-left
  ;; Let's try a few positions
  (let ((click-x 35)
        (click-y 18))
    (test-log "Clicking at (%d, %d) for 'About' link..." click-x click-y)
    (romacs-webkit-click test-view-id click-x click-y 1)
    (test-log "Click sent!")

    ;; Wait and check URL
    (run-at-time 3 nil
      (lambda ()
        (let ((url (romacs-webkit-get-url test-view-id)))
          (test-log "Current URL after click: %s" url)
          (if (string-match-p "about" url)
              (test-log "SUCCESS: Navigated to About page!")
            (test-log "URL doesn't contain 'about' - trying different position..."))
          ;; Exit
          (run-at-time 2 nil (lambda () (kill-emacs 0))))))))

(defun test-run ()
  "Run the test."
  (switch-to-buffer (get-buffer-create "*Click Test*"))
  (erase-buffer)

  (condition-case err
      (progn
        (test-log "Initializing WebKit...")
        (romacs-webkit-init)

        ;; Auto-calculate dimensions to fit window
        (let* ((margin 16)
               (aspect-ratio (/ 16.0 9.0)))
          (setq test-width (- (window-body-width nil t) margin))
          (setq test-height (round (/ test-width aspect-ratio))))

        (test-log "Window size, creating view %dx%d..." test-width test-height)
        (let ((spec (romacs-insert-webkit "https://www.google.com/"
                                            test-width test-height t)))
          (if spec
              (progn
                (setq test-view-id (plist-get (cdr spec) :id))
                (insert (propertize " " 'display spec))
                (test-log "View created with ID: %d" test-view-id)

                ;; Wait for page to load, then click
                (run-at-time 5 nil #'test-click-about))
            (test-log "FAILED: Could not create webkit view"))))
    (error (test-log "ERROR: %S" err)))

  (goto-char (point-min))
  (redisplay t))

(add-hook 'emacs-startup-hook #'test-run)

(provide 'webkit-click-about)
;;; webkit-click-about.el ends here
