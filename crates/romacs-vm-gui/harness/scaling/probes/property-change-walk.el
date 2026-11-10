;;; property-change-walk.el --- scaling probe  -*- lexical-binding: t -*-

;; The buffer walked by next-property-change, as motion over invisible
;; text and redisplay do, where one key reads one value under many stamps
;; (each line's `fontified' put on its own, adjacent, as jit-lock puts its
;; chunks) and another changes every 50 lines.
(scaling-probe property-change-walk :n 20000
  :setup (with-current-buffer (scaling--lines-buffer " scaling-walk" n)
           (goto-char (point-min))
           (dotimes (i n)
             (when (zerop (% i 50)) (put-text-property (point) (line-end-position) 'face 'bold))
             (put-text-property (point) (progn (forward-line 1) (point)) 'fontified t))
           (current-buffer))
  :body (with-current-buffer it
          (let ((pos (point-min)))
            (while (setq pos (next-property-change pos))))
          (let ((pos (point-max)))
            (while (setq pos (previous-property-change pos))))))
