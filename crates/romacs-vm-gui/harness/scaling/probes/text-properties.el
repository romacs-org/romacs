;;; text-properties.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe text-properties :n 2000
  :setup (scaling--lines-buffer " scaling-props" n)
  :body (with-current-buffer it
          (goto-char (point-min))
          (dotimes (i n)
            (put-text-property (point) (line-end-position) 'face (if (cl-oddp i) 'bold 'italic))
            (forward-line 1))
          (goto-char (point-min))
          (dotimes (_ n) (get-text-property (point) 'face) (forward-line 1))
          (let ((pos (point-min)))
            (while (setq pos (next-single-property-change pos 'face))))))
