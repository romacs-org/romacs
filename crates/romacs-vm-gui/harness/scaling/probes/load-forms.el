;;; load-forms.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe load-forms :n 12500 :stack t
  :setup (scaling--file n #'scaling--defun-form)
  :body (load it nil t))
