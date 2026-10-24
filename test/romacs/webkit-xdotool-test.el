;;; webkit-xdotool-test.el --- Test real mouse clicks via xdotool -*- lexical-binding: t -*-

;;; Code:

(require 'romacs-webkit nil t)

(defvar xtest-view-id nil)
(defvar xtest-width nil)
(defvar xtest-height nil)

(defun xtest-log (fmt &rest args)
  "Log to stderr."
  (princ (concat (apply #'format fmt args) "\n") #'external-debugging-output))

(defun xtest-handle-click (event)
  "Handle mouse click EVENT on webkit view."
  (interactive "e")
  (let* ((posn (event-start event))
         (obj (posn-object posn))
         (obj-xy (posn-object-x-y posn)))
    (xtest-log "Mouse click event received!")
    (xtest-log "  posn-object: %S" obj)
    (xtest-log "  posn-object-x-y: %S" obj-xy)
    ;; Check if click is on webkit display spec
    (when (and obj (consp obj) (eq (car obj) 'webkit))
      (let* ((view-id (plist-get (cdr obj) :id))
             (click-x (car obj-xy))
             (click-y (cdr obj-xy)))
        (xtest-log "WebKit click: view=%d x=%d y=%d" view-id click-x click-y)
        (romacs-webkit-click view-id click-x click-y 1)
        (xtest-log "Click forwarded to WebKit!")))))

(defun xtest-handle-scroll (event direction)
  "Handle scroll EVENT in DIRECTION (-1 up, 1 down) on webkit view."
  (let* ((posn (event-start event))
         (obj (posn-object posn))
         (obj-xy (posn-object-x-y posn)))
    ;; Check if scroll is on webkit display spec
    (when (and obj (consp obj) (eq (car obj) 'webkit))
      (let* ((view-id (plist-get (cdr obj) :id))
             (scroll-x (car obj-xy))
             (scroll-y (cdr obj-xy))
             (delta (* direction 50)))  ;; 50 pixels per scroll step
        (xtest-log "WebKit scroll: view=%d x=%d y=%d delta=%d" view-id scroll-x scroll-y delta)
        (romacs-webkit-send-scroll view-id scroll-x scroll-y 0 delta)
        t))))  ;; Return t to indicate we handled it

(defun xtest-handle-scroll-up (event)
  "Handle scroll up EVENT on webkit view."
  (interactive "e")
  (unless (xtest-handle-scroll event -1)
    ;; If not on webkit, let Emacs handle it
    (mwheel-scroll event)))

(defun xtest-handle-scroll-down (event)
  "Handle scroll down EVENT on webkit view."
  (interactive "e")
  (unless (xtest-handle-scroll event 1)
    ;; If not on webkit, let Emacs handle it
    (mwheel-scroll event)))

(defun xtest-run ()
  "Run the test - create webkit view and wait for xdotool click."
  (switch-to-buffer (get-buffer-create "*XDotool Click Test*"))
  (erase-buffer)

  (condition-case err
      (progn
        (xtest-log "=== WebKit XDotool Click Test ===")
        (romacs-webkit-init)

        ;; Auto-calculate dimensions
        (let* ((margin 16)
               (aspect-ratio (/ 16.0 9.0)))
          (setq xtest-width (- (window-body-width nil t) margin))
          (setq xtest-height (round (/ xtest-width aspect-ratio))))

        (xtest-log "Creating webkit view %dx%d..." xtest-width xtest-height)

        (insert "=== WebKit XDotool Click Test ===\n\n")
        (insert "Waiting for real mouse click via xdotool...\n\n")

        (let ((spec (romacs-insert-webkit "https://www.google.com/"
                                            xtest-width xtest-height t)))
          (if spec
              (progn
                (setq xtest-view-id (plist-get (cdr spec) :id))
                (insert (propertize " " 'display spec))
                (insert "\n\n")
                (insert (format "View ID: %d, Size: %dx%d\n"
                               xtest-view-id xtest-width xtest-height))
                (insert "Click will be sent by xdotool in 5 seconds...\n")
                (xtest-log "View created. Waiting for page load...")
                ;; Set up mouse and scroll handlers
                (let ((map (make-sparse-keymap)))
                  (define-key map [mouse-1] #'xtest-handle-click)
                  (define-key map [down-mouse-1] #'ignore)
                  (define-key map [wheel-up] #'xtest-handle-scroll-up)
                  (define-key map [wheel-down] #'xtest-handle-scroll-down)
                  (define-key map [mouse-4] #'xtest-handle-scroll-up)
                  (define-key map [mouse-5] #'xtest-handle-scroll-down)
                  (use-local-map map))
                (xtest-log "Mouse and scroll handlers installed"))
            (xtest-log "FAILED: Could not create webkit view"))))
    (error (xtest-log "ERROR: %S" err)))

  (goto-char (point-min))
  (redisplay t)

  ;; Keep running for xdotool test
  (xtest-log "Emacs ready. Waiting 30 seconds for xdotool interaction...")
  (run-at-time 30 nil (lambda ()
                        (xtest-log "Test timeout - exiting")
                        (kill-emacs 0))))

(add-hook 'emacs-startup-hook #'xtest-run)

(provide 'webkit-xdotool-test)
;;; webkit-xdotool-test.el ends here
