;;; gc-retired-code.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe gc-retired-code :gui t :n 1000 :steps 4 :gap 1.0
  :setup (progn
           (defalias 'romacs--idle-collect (lambda (&optional _in-lisp) t))
           (setq scaling--retire-collections gcs-done)
           (switch-to-buffer (get-buffer-create "*scaling-retire*")))
  :body (progn
          (when (and (> i 0) (<= gcs-done scaling--retire-collections))
            (kill-emacs 3))
          (dotimes (k n)
            (funcall (eval (list 'lambda nil (list 'list (list '+ k i) k '(symbol-name 'scaling))) t)))
          (with-current-buffer (window-buffer (selected-window))
            (goto-char (point-max))
            (insert (format "step %d\n" i)))
          (message nil)
          (setq scaling--retire-collections gcs-done)))
