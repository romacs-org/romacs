;;; theme-bundled-rows.el --- scaling probe  -*- lexical-binding: t -*-

;; Registering N bundled themes, as a load of romacs-theme.el does: a property
;; each, no scan of the table per theme.
(scaling-probe theme-bundled-rows :n 4000
  :setup (progn
           (require 'romacs-theme)
           (mapcar (lambda (name) (list (intern name) 'dark (intern (concat name "-family"))))
                   (scaling--names "scaling-theme" n)))
  :body (romacs-theme--register it))
