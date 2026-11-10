;;; theme-floor-rows.el --- scaling probe  -*- lexical-binding: t -*-

;; The floor pass and its merge over N face rows, a tenth of them failing on
;; one pair: each pair is computed once, each row merged once.
(scaling-probe theme-floor-rows :n 4000
  :setup (progn
           (require 'romacs-theme-roles)
           (mapcar (lambda (name)
                     (list (intern name) (list (list t :foreground (if (string-suffix-p "1" name) "#333333" "#eeeeee")))))
                   (scaling--names "scaling-floor" n)))
  :body (let ((moves (romacs-theme-roles-floors it '((bg . "#101418")))))
          (romacs-theme--apply-delta it (mapcar (lambda (move) (list (car move) :foreground (nth 2 move))) moves))))
