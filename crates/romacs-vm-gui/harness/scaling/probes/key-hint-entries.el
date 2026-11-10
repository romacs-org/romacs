;;; key-hint-entries.el --- scaling probe  -*- lexical-binding: t -*-

;; N Omarchy themes Romacs does not ship, made one after another, the first
;; made again: Custom's theme list and `features' grow by one each.  Sessions
;; meet tens of them.
;; Each distinct hint made once, then found again, the oldest first.
(scaling-probe key-hint-entries :n 2000
  :setup (progn (require 'romacs-key-hint) (scaling--names "\\`C-x' hint" n))
  :body (progn (dolist (text it) (romacs-key-hint text))
               (dolist (text it) (romacs-key-hint text))))
