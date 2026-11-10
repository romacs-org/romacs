;;; hscroll-faces-on-line.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe hscroll-faces-on-line :gui t :n 500 :steps 40
  :setup (progn (switch-to-buffer (get-buffer-create "hs")) (erase-buffer)
                (dotimes (i n) (let ((face (intern (format "scaling-hs-%d" i))))
                                 (custom-declare-face face '((t :weight bold)) "x" :group 'scaling)
                                 (insert (propertize "x" 'face face))))
                (insert "\n") (setq-local truncate-lines t) (goto-char 10)
                (current-buffer))
  :body (with-current-buffer it (if (cl-evenp i) (forward-char 1) (backward-char 1))))
