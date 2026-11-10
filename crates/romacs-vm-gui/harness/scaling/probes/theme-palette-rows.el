;;; theme-palette-rows.el --- scaling probe  -*- lexical-binding: t -*-

;; A theme's role resolution over N more palette rows: an index, not a scan
;; per name.
(scaling-probe theme-palette-rows :n 2000
  :setup (progn
           (require 'romacs-theme-roles)
           (append '((mode dark) (bg "#101418") (fg "#d8dee9") (red "#e06c75") (green "#98c379")
                     (yellow "#e5c07b") (blue "#61afef") (magenta "#c678dd") (cyan "#56b6c2"))
                   (mapcar (lambda (name) (list (intern name) "#123456")) (scaling--names "scaling-slot" n))))
  :body (romacs-theme-roles-resolve it))
