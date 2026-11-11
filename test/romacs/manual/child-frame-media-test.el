;;; child-frame-media-test.el --- Test image/video inside child frames -*- lexical-binding: t -*-

;; Verify that child frames can render inline images and videos
;; via the shared render_frame_content() path.
;; Usage: emacs -Q -l test/romacs/manual/child-frame-media-test.el

;;; Code:

(require 'romacs-video nil t)

(defvar cfm-test--frames nil "Child frames created during test.")

(defun cfm-test--cleanup ()
  "Delete all test child frames."
  (dolist (f cfm-test--frames)
    (when (frame-live-p f) (delete-frame f)))
  (setq cfm-test--frames nil))

(defun cfm-test--make-child (name x y w h &optional params)
  "Create a child frame NAME at (X,Y) size (W,H) with extra PARAMS."
  (let* ((buf (get-buffer-create (format "*cfm-%s*" name)))
         (frame (make-frame
                 (append
                  `((parent-frame . ,(selected-frame))
                    (left . ,x) (top . ,y)
                    (width . ,w) (height . ,h)
                    (minibuffer . nil)
                    (no-accept-focus . nil)
                    (child-frame-border-width . 2)
                    (internal-border-width . 4)
                    (undecorated . t)
                    (visibility . t))
                  params))))
    (with-selected-frame frame
      (switch-to-buffer buf)
      (erase-buffer))
    (push frame cfm-test--frames)
    frame))

(defun cfm-test--log (fmt &rest args)
  "Log to stderr."
  (princ (concat (apply #'format fmt args) "\n") #'external-debugging-output))

(defun cfm-test--insert-video-display (file width height)
  "Insert FILE as a raw video display property with WIDTH and HEIGHT."
  (insert (propertize " "
                      'display `(video :file ,file
                                       :width ,width
                                       :height ,height
                                       :loop-count -1
                                       :autoplay t))))

;; ============================================================
;; Test: Image in child frame
;; ============================================================

(defun cfm-test--image ()
  "Create a child frame containing an inline image."
  (cfm-test--cleanup)
  (let* ((img-path (or (let ((p (expand-file-name "~/Pictures/4k_image_1.jpg")))
                         (and (file-exists-p p) p))
                       (expand-file-name "etc/images/splash.png")))
         (f (cfm-test--make-child "image" 50 30 60 20)))
    (with-selected-frame f
      (insert (propertize "Image in Child Frame\n"
                          'face '(:foreground "gold" :weight bold :height 1.2)))
      (insert (format "File: %s\n\n" (file-name-nondirectory img-path)))
      (condition-case err
          (let ((img (create-image img-path nil nil
                                   :max-width 400 :max-height 250)))
            (if img
                (progn
                  (insert-image img "[IMAGE]")
                  (insert "\n\n")
                  (insert (propertize "Image rendered successfully!"
                                      'face '(:foreground "lime green")))
                  (cfm-test--log "Image: OK (%s)" (file-name-nondirectory img-path)))
              (insert "[create-image returned nil]\n")
              (cfm-test--log "Image: FAILED")))
        (error
         (insert (format "[Error: %S]\n" err))
         (cfm-test--log "Image: ERROR %S" err))))
    (message "Child frame image test — press 'v' for video, 'b' for both, 'q' to quit")))

;; ============================================================
;; Test: Video in child frame
;; ============================================================

(defun cfm-test--video ()
  "Create a child frame containing an inline video."
  (cfm-test--cleanup)
  (let* ((vid-path (let ((p (expand-file-name "~/Videos/test.mp4")))
                     (and (file-exists-p p) p)))
         (f (cfm-test--make-child "video" 50 30 60 20)))
    (with-selected-frame f
      (insert (propertize "Video in Child Frame\n"
                          'face '(:foreground "cyan" :weight bold :height 1.2)))
      (if vid-path
          (progn
            (insert (format "File: %s\n\n" (file-name-nondirectory vid-path)))
            (condition-case err
                (progn
                  (cfm-test--insert-video-display vid-path 400 250)
                  (insert "\n\n")
                  (insert (propertize "Video display property looping"
                                      'face '(:foreground "lime green")))
                  (cfm-test--log "Video: OK (display property)"))
              (error
               (insert (format "[Error: %S]\n" err))
               (cfm-test--log "Video: ERROR %S" err))))
        (insert "[No video file found]\n")
        (cfm-test--log "Video: SKIPPED (no file)")))
    (message "Child frame video test — press 'i' for image, 'b' for both, 'q' to quit")))

;; ============================================================
;; Test: Both image and video in separate child frames
;; ============================================================

(defun cfm-test--both ()
  "Create two child frames: one with image, one with video."
  (cfm-test--cleanup)
  (let* ((img-path (or (let ((p (expand-file-name "~/Pictures/4k_image_1.jpg")))
                         (and (file-exists-p p) p))
                       (expand-file-name "etc/images/splash.png")))
         (vid-path (let ((p (expand-file-name "~/Videos/test.mp4")))
                     (and (file-exists-p p) p)))
         (f-img (cfm-test--make-child "img" 30 30 55 18))
         (f-vid (cfm-test--make-child "vid" 480 320 55 18)))
    ;; Image child frame
    (with-selected-frame f-img
      (insert (propertize "Image\n" 'face '(:foreground "gold" :weight bold)))
      (condition-case err
          (let ((img (create-image img-path nil nil
                                   :max-width 350 :max-height 220)))
            (when img
              (insert-image img "[IMG]")
              (insert "\n")
              (cfm-test--log "Both/Image: OK")))
        (error (cfm-test--log "Both/Image: ERROR %S" err))))
    ;; Video child frame
    (with-selected-frame f-vid
      (insert (propertize "Video\n" 'face '(:foreground "cyan" :weight bold)))
      (if vid-path
          (condition-case err
              (progn
                (cfm-test--insert-video-display vid-path 350 220)
                (insert "\n")
                (cfm-test--log "Both/Video: OK (display property)"))
            (error (cfm-test--log "Both/Video: ERROR %S" err)))
        (insert "[No video]\n")
        (cfm-test--log "Both/Video: SKIPPED")))
    (message "Both child frames — press 'i' for image only, 'v' for video only, 'q' to quit")))

;; ============================================================
;; Key bindings and auto-start
;; ============================================================

(defvar cfm-test-map (make-sparse-keymap))
(define-key cfm-test-map (kbd "i") (lambda () (interactive) (cfm-test--image)))
(define-key cfm-test-map (kbd "v") (lambda () (interactive) (cfm-test--video)))
(define-key cfm-test-map (kbd "b") (lambda () (interactive) (cfm-test--both)))
(define-key cfm-test-map (kbd "q") (lambda () (interactive)
                                     (cfm-test--cleanup)
                                     (message "Cleaned up.")))
(set-transient-map cfm-test-map t)

;; Resize frame for side-by-side child frames
(set-frame-size (selected-frame) 120 50)

(message "=== Child Frame Media Test ===")
(message "Keys: i=image  v=video  b=both  q=quit")
(cfm-test--both)

;;; child-frame-media-test.el ends here
