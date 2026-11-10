;;; display-appended-tail.el --- scaling probe  -*- lexical-binding: t -*-

;; The same with the window at the end, its lines numbered.
(scaling-probe display-appended-tail :gui t :n 20000 :k 8 :steps 20 :budget 0.1
  :setup (let ((buffer (scaling--lines-buffer "scaling-appended-tail" n)))
           (switch-to-buffer buffer)
           (display-line-numbers-mode 1)
           (goto-char (point-max))
           (forward-line -30)
           buffer)
  :body (with-current-buffer it
          (save-excursion (goto-char (point-max)) (insert "an appended line\n"))
          (forward-char 1)))
