;;; redisplay-lines.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe redisplay-lines :gui t :n 2000 :steps 20 :budget 0.5
  :setup (let ((buffer (scaling--lines-buffer "scaling-lines" n)))
           (with-current-buffer buffer
             (goto-char (point-min))
             (dotimes (i n)
               (put-text-property (point) (line-end-position) 'face (if (cl-oddp i) 'bold 'font-lock-keyword-face))
               (forward-line 1))
             (goto-char (/ (point-max) 2)))
           (switch-to-buffer buffer)
           buffer)
  :body (with-current-buffer it (forward-line 1) (insert "x")))
