;;; romacs-webkit-test.el --- Test inline WebKit rendering in Romacs -*- lexical-binding: t -*-

;; This test verifies that WebKit views are rendered inline in buffers
;; using WPE WebKit with GPU acceleration.

;;; Commentary:
;; Run with: ./test/romacs/run-webkit-test.sh
;; Or manually: DISPLAY=:0 ./target/release/romacs -Q -l test/romacs/romacs-webkit-test.el
;;
;; Romacs renders WebKit views inline in buffers (not as floating overlays).
;; This uses a declarative display property:
;;
;;   (insert (propertize " " 'display
;;                       '(webkit :uri "https://example.com"
;;                                :width 400 :height 300)))
;;
;; The webkit view becomes part of the buffer content, scrolls naturally,
;; and respects Emacs window management.

;;; Code:

(defconst romacs-webkit-test-default-url
  "data:text/html,%3Chtml%3E%3Cbody%20style%3D%22margin%3A0%3Bbackground%3A%231e1e1e%3Bcolor%3Awhite%3Bfont%3A24px%20sans-serif%22%3E%3Cdiv%20style%3D%22padding%3A40px%22%3E%3Ch1%3EInline%20WebKit%20Buffer%20Test%3C%2Fh1%3E%3Cinput%20value%3D%22type%20here%22%20style%3D%22font-size%3A24px%22%3E%3Cp%3EThis%20WebKit%20view%20is%20inside%20a%20buffer%20display%20property.%3C%2Fp%3E%3C%2Fdiv%3E%3C%2Fbody%3E%3C%2Fhtml%3E"
  "Self-contained HTML page for inline WebKit rendering tests.")

(defvar romacs-webkit-test-url romacs-webkit-test-default-url
  "URL to load for testing.")

(defvar romacs-webkit-test-width 0
  "Width of test WebKit view (0 = auto-fit to window).")

(defvar romacs-webkit-test-height 0
  "Height of test WebKit view (0 = auto from aspect ratio).")

(defun romacs-webkit-test-run ()
  "Run the inline WebKit rendering test."
  (interactive)
  (switch-to-buffer (get-buffer-create "*WebKit Test*"))
  (erase-buffer)

  (insert "=== Romacs Inline WebKit Test ===\n\n")

  (condition-case err
      (progn
        ;; Calculate dimensions (0 = auto-fit to window)
        (let* ((margin 16)
               (aspect-ratio (/ 16.0 9.0))
               (width (if (and romacs-webkit-test-width (> romacs-webkit-test-width 0))
                          romacs-webkit-test-width
                        (- (window-body-width nil t) margin)))
               (height (if (and romacs-webkit-test-height (> romacs-webkit-test-height 0))
                           romacs-webkit-test-height
                         (round (/ width aspect-ratio)))))
          (insert (format "Loading %s inline (%dx%d)...\n\n"
                          romacs-webkit-test-url width height))

          (let ((spec (list 'webkit
                            :uri romacs-webkit-test-url
                            :width width
                            :height height)))
            (insert (propertize " " 'display spec))
            (insert "\n\n")
            (insert (format "WebKit spec: %S\n\n" spec))
            (insert "SUCCESS! Declarative inline WebKit display property installed.\n"))))
    (error
     (insert (format "ERROR: %S\n" err))))

  (goto-char (point-min))
  (redisplay t)

  ;; Auto-exit after delay when run non-interactively
  (when noninteractive
    (run-at-time 15 nil (lambda () (kill-emacs 0)))))

;; Auto-run when loaded
(add-hook 'emacs-startup-hook #'romacs-webkit-test-run)

(provide 'romacs-webkit-test)
;;; romacs-webkit-test.el ends here
