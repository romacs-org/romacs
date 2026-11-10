;;; markers.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe markers :n 2000
  :setup (with-current-buffer (scaling--lines-buffer " scaling-markers" 200) (current-buffer))
  :body (with-current-buffer it
          (let ((size (buffer-size)) markers)
            (dotimes (i n) (push (copy-marker (1+ (% (* i 7919) size))) markers))
            (dotimes (i 100) (goto-char (1+ (% (* i 104729) (buffer-size)))) (insert "x"))
            (dolist (m markers) (marker-position m))
            (dolist (m markers) (set-marker m nil)))))
