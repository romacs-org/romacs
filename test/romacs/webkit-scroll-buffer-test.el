;;; webkit-scroll-buffer-test.el --- Test buffer scrolling with inline webkit -*- lexical-binding: t -*-

;;; Code:

(require 'romacs-webkit nil t)

(defun scroll-test-log (fmt &rest args)
  (princ (concat (apply #'format fmt args) "\n") #'external-debugging-output))

(defun scroll-test-run ()
  "Test buffer scrolling with inline webkit view."
  (switch-to-buffer (get-buffer-create "*Scroll Test*"))
  (erase-buffer)

  (scroll-test-log "=== WebKit Buffer Scroll Test ===")

  (condition-case err
      (progn
        (romacs-webkit-init)

        ;; Add many lines of text ABOVE the webkit view
        (insert "=== WebKit Buffer Scroll Test ===\n\n")
        (dotimes (i 20)
          (insert (format "Line %02d above webkit view - this text should scroll normally\n" (1+ i))))

        (insert "\n--- WEBKIT VIEW BELOW ---\n\n")

        ;; Insert webkit view
        (let* ((width 500)
               (height 300)
               (spec (romacs-insert-webkit "https://www.google.com/" width height t)))
          (if spec
              (progn
                (insert (propertize " " 'display spec))
                (scroll-test-log "WebKit view inserted"))
            (insert "[WEBKIT FAILED]\n")))

        (insert "\n\n--- TEXT BELOW WEBKIT ---\n\n")

        ;; Add many lines of text BELOW the webkit view
        (dotimes (i 30)
          (insert (format "Line %02d below webkit view - scroll down to see this\n" (1+ i))))

        (insert "\n=== END OF BUFFER ===\n"))
    (error (scroll-test-log "ERROR: %S" err)))

  (goto-char (point-min))
  (scroll-test-log "Buffer ready. Use C-n to scroll down and test webkit positioning.")
  (scroll-test-log "Waiting 60 seconds for testing...")

  ;; Keep running for manual testing
  (run-at-time 60 nil (lambda () (kill-emacs 0))))

(add-hook 'emacs-startup-hook #'scroll-test-run)

(provide 'webkit-scroll-buffer-test)
;;; webkit-scroll-buffer-test.el ends here
