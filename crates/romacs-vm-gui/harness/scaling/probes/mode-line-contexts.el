;;; mode-line-contexts.el --- scaling probe  -*- lexical-binding: t -*-

;; N side windows whose buffers set their own mode line, each folded into
;; the status bar as a context segment (VM2-STATUS-BAR), a key typed in
;; the selected window.
(scaling-probe mode-line-contexts :gui t :n 2 :steps 20
  :setup (progn
           (delete-other-windows)
           (let ((main (scaling--lines-buffer "scaling-contexts" 20)))
             (switch-to-buffer main)
             (dotimes (i n)
               (let ((buffer (scaling--lines-buffer (format "scaling-context-%d" i) 2)))
                 (with-current-buffer buffer (setq-local mode-line-format (list (format "ctx %d " i) "%b")))
                 (display-buffer-in-side-window buffer `((side . right) (slot . ,i) (window-width . 12)))))
             main))
  :body (with-current-buffer it (insert "x")))
