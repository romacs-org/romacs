;;; face-remap-bar-unchanged.el --- scaling probe  -*- lexical-binding: t -*-

;; The same remapping in the selected window, the bar's owner, while the
;; keys go to a buffer in the other: the bar's line is unchanged, so each
;; request finds its last expansion by its key, which reads the remapping
;; again (`bar_key').
(scaling-probe face-remap-bar-unchanged :gui t :n 256 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (let ((other (scaling--lines-buffer "scaling-remap-other" 50)))
             (switch-to-buffer (scaling--lines-buffer "scaling-remap-still" 50))
             (setq-local face-remapping-alist
                         (cl-loop for i below n
                                  collect (let ((face (intern (format "remap-still-face-%d" i))))
                                            (make-face face)
                                            (list face (list :foreground (format "#%02x6080" (% (* 7 i) 256)))))))
             (set-window-buffer (split-window-below) other)
             other))
  :body (with-current-buffer it (insert "x")))
