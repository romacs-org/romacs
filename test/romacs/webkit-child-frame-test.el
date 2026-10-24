;;; webkit-child-frame-test.el --- Test WebKit view inside a child frame -*- lexical-binding: t -*-

;; Open a WPE WebKit view showing YouTube inside a child frame.
;; Usage: ./target/release/romacs -Q -l test/romacs/webkit-child-frame-test.el

;;; Code:

(require 'romacs-webkit nil t)

(defvar wkcf-test--frame nil
  "The child frame hosting the WebKit view.")

(defun wkcf-test--cleanup ()
  "Delete the WebKit child frame."
  (when (and wkcf-test--frame (frame-live-p wkcf-test--frame))
    (delete-frame wkcf-test--frame))
  (setq wkcf-test--frame nil))

(defun wkcf-test-run ()
  "Create a child frame with a WebKit view showing YouTube."
  (interactive)
  (wkcf-test--cleanup)

  ;; Style child frame
  (when (fboundp 'romacs-set-child-frame-style)
    (romacs-set-child-frame-style
     :corner-radius 12 :shadow t
     :shadow-layers 6 :shadow-offset 3 :shadow-opacity 35))

  ;; Calculate a large child frame centered in the parent
  (let* ((pw (frame-pixel-width))
         (ph (frame-pixel-height))
         (cw (min (- pw 80) 1200))
         (ch (min (- ph 100) 800))
         (cx (/ (- pw cw) 2))
         (cy (/ (- ph ch) 2))
         ;; Convert pixel size to character cells
         (char-w (/ cw (frame-char-width)))
         (char-h (/ ch (frame-char-height)))
         (buf (get-buffer-create "*webkit-child-frame*"))
         (frame (make-frame
                 `((parent-frame . ,(selected-frame))
                   (left . ,cx) (top . ,cy)
                   (width . ,char-w) (height . ,char-h)
                   (minibuffer . nil)
                   (no-accept-focus . nil)
                   (child-frame-border-width . 4)
                   (internal-border-width . 4)
                   (undecorated . t)
                   (visibility . t)
                   (alpha-background . 100)))))

    (setq wkcf-test--frame frame)

    (with-selected-frame frame
      (switch-to-buffer buf)
      (erase-buffer)
      ;; Make child-frame border visible
      (set-face-background 'child-frame-border "#5577cc" frame)
      (set-face-background 'internal-border "#333344" frame)

      (condition-case err
          (progn
            (romacs-webkit-init)
            ;; Use full child-frame window dimensions for the WebKit view
            (let* ((wk-w (window-body-width nil t))
                   (wk-h (window-body-height nil t))
                   (spec (romacs-insert-webkit "https://www.youtube.com"
                                                wk-w wk-h t)))
              (if spec
                  (let ((view-id (plist-get (cdr spec) :id)))
                    (insert (propertize " " 'display spec
                                        'romacs-webkit-id view-id))
                    (romacs-webkit--register-view (1- (point)) view-id)
                    (romacs-webkit-enable-auto-resize)
                    (goto-char (point-min))
                    (message "WebKit child frame: YouTube loaded (view %d, %dx%d)"
                             view-id wk-w wk-h))
                (insert "Failed: romacs-insert-webkit returned nil\n"))))
        (error
         (insert (format "Error: %S\n" err)))))

    (raise-frame frame))

  (message "webkit-child-frame-test: q=close  r=reload"))

;; Keybindings
(defvar wkcf-test-map (make-sparse-keymap))
(define-key wkcf-test-map (kbd "q") (lambda () (interactive)
                                      (wkcf-test--cleanup)
                                      (message "WebKit child frame closed.")))
(define-key wkcf-test-map (kbd "r") (lambda () (interactive) (wkcf-test-run)))
(set-transient-map wkcf-test-map t)

;; Auto-run on load
(wkcf-test-run)

;;; webkit-child-frame-test.el ends here
