;;; mode-line-windows.el --- scaling probe  -*- lexical-binding: t -*-

;; N windows, each showing its own buffer and drawing its own line
;; (VM2-MODE-LINE), a key typed in one: the others' lines are reused,
;; not expanded again.
(scaling-probe mode-line-windows :gui t :n 2 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (delete-other-windows)
           (let ((main (scaling--lines-buffer "scaling-windows" 20)))
             (switch-to-buffer main)
             (save-selected-window
               (dotimes (i (1- n))
                 (select-window (split-window (get-largest-window) nil (quote below)))
                 (switch-to-buffer (scaling--lines-buffer (format "scaling-window-%d" i) 5))))
             (balance-windows)
             main))
  :body (with-current-buffer it (insert "x")))
