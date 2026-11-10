;;; terminal-face-tables-own.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe terminal-face-tables-own :gui t :n 4 :steps 4 :budget 0.25
  :setup (scaling--terminals n)
  :body (with-current-buffer it
          (let ((inhibit-read-only t)) (goto-char (point-max)) (insert "x"))))
