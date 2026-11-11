;;; overlays.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe overlays :n 1000
  :setup (scaling--lines-buffer " scaling-overlays" n)
  :body (with-current-buffer it
          (let (overlays)
            (goto-char (point-min))
            (dotimes (_ n)
              (let ((o (make-overlay (point) (line-end-position))))
                (overlay-put o 'face 'bold)
                (push o overlays))
              (forward-line 1))
            (goto-char (point-min))
            (dotimes (_ n)
              (overlays-at (point))
              (overlays-in (point) (line-end-position))
              (forward-line 1))
            (dolist (o overlays) (move-overlay o (overlay-start o) (1+ (overlay-start o))))
            (mapc #'delete-overlay overlays))))
