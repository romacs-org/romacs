;;; terminal-face-tables-none.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe terminal-face-tables-none :gui t :n 4 :steps 4 :budget 0.25
  :setup (progn (scaling--terminals n) (switch-to-buffer (get-buffer-create "scaling-typing")))
  :body (with-current-buffer it (insert "x")))
