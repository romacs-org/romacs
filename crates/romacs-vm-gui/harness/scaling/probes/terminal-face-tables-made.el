;;; terminal-face-tables-made.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe terminal-face-tables-made :gui t :n 4 :steps 4 :budget 0.25
  :setup (scaling--terminals n)
  :body (with-current-buffer it
          (set-face-attribute 'highlight nil :weight (if (= (% i 2) 0) 'bold 'normal))
          (let ((inhibit-read-only t)) (goto-char (point-max)) (insert "x"))))
