;;; gc-retired-code.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe gc-retired-code :gui t :n 1000 :steps 4 :gap 1.0
  ;; Each step after a collection since the last: the host's idle pass
  ;; collects only when no timer is due, which a loaded host may not reach
  ;; within a fixed gap.
  :ready (> gcs-done scaling--retire-collections)
  :setup (progn
           (defalias 'romacs--idle-collect (lambda (&optional _in-lisp) t))
           (setq scaling--retire-collections gcs-done)
           (switch-to-buffer (get-buffer-create "*scaling-retire*")))
  :body (progn
          (dotimes (k n)
            (funcall (eval (list 'lambda nil (list 'list (list '+ k i) k '(symbol-name 'scaling))) t)))
          (with-current-buffer (window-buffer (selected-window))
            (goto-char (point-max))
            (insert (format "step %d\n" i)))
          (message nil)
          (setq scaling--retire-collections gcs-done)))
