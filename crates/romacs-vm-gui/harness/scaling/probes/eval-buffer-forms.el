;;; eval-buffer-forms.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe eval-buffer-forms :n 12500 :stack t
  :setup (let ((file (scaling--file n #'scaling--defun-form)))
           (with-current-buffer (get-buffer-create " scaling-eval-buffer")
             (insert-file-contents file)
             (current-buffer)))
  :body (eval-buffer it))
