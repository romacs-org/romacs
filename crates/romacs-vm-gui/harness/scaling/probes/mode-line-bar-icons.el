;;; mode-line-bar-icons.el --- scaling probe  -*- lexical-binding: t -*-

;; N characters the bar draws as an icon an alist of N entries names,
;; the icon's entry at the end of a registry grown by N: the registry and
;; each piece's alist are indexed once, not walked per character.
(scaling-probe mode-line-bar-icons :gui t :n 20 :steps 20
  :setup (progn
           (require 'romacs-icons)
           (setq romacs-icons--font t romacs-icons-source 'romacs romacs-icons-ascii nil)
           (setq romacs-icons-alist
                 (append romacs-icons-alist
                         (cl-loop for j below n collect (list (intern (format "probe-icon-%d" j)) #xFE001))))
           ;; The string's own mapping grows too: N characters, each mapped
           ;; near its end.
           (let ((last (intern (format "probe-icon-%d" (1- n))))
                 (text (apply #'string (cl-loop for j below n collect (+ #x100 j)))))
             (setq-default mode-line-format
                           `((:propertize ,text romacs-icon
                                          ,(cl-loop for j below n collect (cons (+ #x100 j) last))))))
           (switch-to-buffer (scaling--lines-buffer "scaling-bar-icons" 50)))
  :body (with-current-buffer it (insert "x")))
