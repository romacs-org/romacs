;;; webkit-mouse-test.el --- Test inline WebKit mouse click handling -*- lexical-binding: t -*-

;;; Commentary:
;; Test script to verify mouse clicks work on inline WebKit views.
;; Run with: DISPLAY=:0 ./target/release/romacs -Q -l test/romacs/webkit-mouse-test.el

;;; Code:

(require 'romacs-webkit nil t)

(defvar webkit-mouse-test-view-id nil
  "View ID for the test webkit view.")

(defvar webkit-mouse-test-view-pos nil
  "Buffer position where the webkit view starts.")

(defvar webkit-mouse-test-view-width 0
  "Width of the test webkit view.")

(defvar webkit-mouse-test-view-height 0
  "Height of the test webkit view.")

(defun webkit-mouse-test--handle-click (event)
  "Handle mouse click EVENT on inline webkit view."
  (interactive "e")
  (let* ((posn (event-start event))
         (obj (posn-object posn))
         (pos (posn-point posn))
         (obj-xy (posn-object-x-y posn))
         (win-xy (posn-x-y posn)))
    (message "Click at pos=%s obj=%S obj-xy=%S win-xy=%S"
             pos obj obj-xy win-xy)
    ;; Check if we clicked on a webkit display spec
    (when obj
      (let ((display-spec (if (consp obj) (car obj) obj)))
        (when (and (consp display-spec)
                   (eq (car display-spec) 'webkit))
          (let* ((view-id (plist-get (cdr display-spec) :id))
                 (view-x (car obj-xy))
                 (view-y (cdr obj-xy)))
            (message "WebKit click: view-id=%d x=%d y=%d" view-id view-x view-y)
            ;; Send click to webkit
            (when (fboundp 'romacs-webkit-click)
              (romacs-webkit-click view-id view-x view-y 1)
              (message "Sent click to WebKit view %d at (%d, %d)" view-id view-x view-y))))))))

(defun webkit-mouse-test--setup-keymap ()
  "Set up keymap to capture mouse clicks."
  (let ((map (make-sparse-keymap)))
    (define-key map [mouse-1] #'webkit-mouse-test--handle-click)
    (define-key map [down-mouse-1] #'webkit-mouse-test--handle-click)
    (use-local-map map)))

(defun webkit-mouse-test-run ()
  "Run the webkit mouse click test."
  (interactive)
  (switch-to-buffer (get-buffer-create "*WebKit Mouse Test*"))
  (erase-buffer)

  (insert "=== WebKit Mouse Click Test ===\n\n")
  (insert "Click on the webpage below to test mouse input.\n\n")

  (condition-case err
      (progn
        (romacs-webkit-init)
        (insert "WebKit initialized.\n\n")

        ;; Calculate dimensions
        (let* ((margin 16)
               (width (- (window-body-width nil t) margin))
               (height (round (/ width 1.5))))
          (insert (format "Creating view %dx%d...\n\n" width height))

          ;; Create webkit view - load a simple clickable page
          (let ((spec (romacs-insert-webkit "https://www.google.com/"
                                              width height t)))
            (if spec
                (progn
                  (setq webkit-mouse-test-view-id (plist-get (cdr spec) :id))
                  (setq webkit-mouse-test-view-pos (point))
                  (setq webkit-mouse-test-view-width width)
                  (setq webkit-mouse-test-view-height height)
                  (insert (propertize " " 'display spec))
                  (insert "\n\n")
                  (insert (format "View ID: %d\n" webkit-mouse-test-view-id))
                  (insert "Mouse click handling enabled.\n")
                  (insert "Try clicking on links in the webpage above.\n\n")

                  ;; Setup mouse handling
                  (webkit-mouse-test--setup-keymap)
                  (message "WebKit mouse test ready - click on the view!")
                  ;; Auto-test after delay
                  (webkit-mouse-test--auto-click))
              (insert "FAILED: romacs-insert-webkit returned nil\n")))))
    (error
     (insert (format "ERROR: %S\n" err))))

  (goto-char (point-min))
  (redisplay t))

(defun webkit-mouse-test--log (fmt &rest args)
  "Log FMT with ARGS to stderr."
  (let ((msg (apply #'format fmt args)))
    (princ (concat msg "\n") #'external-debugging-output)))

(defun webkit-mouse-test--auto-click ()
  "Automatically test click detection after a short delay."
  (run-at-time 3 nil
    (lambda ()
      (webkit-mouse-test--log "=== Auto-testing webkit click detection ===")
      ;; Go to the webkit position
      (when webkit-mouse-test-view-pos
        (goto-char webkit-mouse-test-view-pos)
        ;; Get posn-at-point info
        (let* ((posn (posn-at-point))
               (obj (posn-object posn))
               (obj-xy (posn-object-x-y posn)))
          (webkit-mouse-test--log "posn-at-point: %S" posn)
          (webkit-mouse-test--log "posn-object: %S" obj)
          (webkit-mouse-test--log "posn-object-x-y: %S" obj-xy)
          ;; Check if we got webkit object
          (if (and obj (consp obj) (eq (car obj) 'webkit))
              (progn
                (webkit-mouse-test--log "SUCCESS: Got webkit object from posn!")
                (let ((view-id (plist-get (cdr obj) :id)))
                  (webkit-mouse-test--log "view-id from posn: %S" view-id)
                  ;; Try sending a click to the center
                  (when (and view-id webkit-mouse-test-view-width webkit-mouse-test-view-height)
                    (let ((click-x (/ webkit-mouse-test-view-width 2))
                          (click-y (/ webkit-mouse-test-view-height 2)))
                      (webkit-mouse-test--log "Sending test click to webkit view %d at (%d, %d)"
                               view-id click-x click-y)
                      (romacs-webkit-click view-id click-x click-y 1)
                      (webkit-mouse-test--log "Click sent!")))))
            (webkit-mouse-test--log "FAILED: posn-object did not return webkit spec")))
        ;; Exit after test
        (run-at-time 2 nil (lambda () (kill-emacs 0)))))))

;; Auto-run
(add-hook 'emacs-startup-hook #'webkit-mouse-test-run)

(provide 'webkit-mouse-test)
;;; webkit-mouse-test.el ends here
