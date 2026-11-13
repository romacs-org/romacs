;;; display-overlay-elsewhere.el --- scaling probe  -*- lexical-binding: t -*-

;; Overlays changing in buffers no window shows, as a background mode's
;; highlighting does, while the one window on another buffer is
;; redisplayed with point moved: the window reads its own buffer's overlay
;; revision, so a step costs the same however many buffers hold overlays.
;; Each step puts a property on, moves, deletes and makes again eight of
;; them.  What this holds is that no overlay revision is found by a walk
;; of the buffers; that the window is reused, not rebuilt, is a constant
;; the exponent cannot see, and romacs-vm's window tests count it.
(scaling-probe display-overlay-elsewhere :gui t :n 50 :k 8 :steps 20 :budget 0.1
  :setup (let (overlays)
           (dotimes (i n)
             (with-current-buffer (get-buffer-create (format " scaling-overlay-%d" i))
               (insert "some text for an overlay to cover\n")
               (let ((overlay (make-overlay 1 5)))
                 (overlay-put overlay 'face 'highlight)
                 (push overlay overlays))))
           (cons (switch-to-buffer (scaling--lines-buffer "scaling-overlay-shown" 2000))
                 (vconcat overlays)))
  :body (let ((overlays (cdr it)))
          (dotimes (j 8)
            (let* ((at (% (+ (* i 8) j) n))
                   (overlay (aref overlays at))
                   (buffer (overlay-buffer overlay)))
              (overlay-put overlay 'priority i)
              (move-overlay overlay (+ 2 (% i 3)) 9)
              (delete-overlay overlay)
              (aset overlays at (setq overlay (make-overlay 1 5 buffer)))
              (overlay-put overlay 'face 'highlight)))
          (with-current-buffer (car it) (forward-char 1))))
