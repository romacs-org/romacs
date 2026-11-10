;;; theme-pill-stack.el --- scaling probe  -*- lexical-binding: t -*-

;; The pill's colours under N theme entries on each of its faces: the entries
;; are walked once, by the face's recalculation.
(scaling-probe theme-pill-stack :n 2000 :k 4
  :setup (progn
           (require 'romacs-mode-line)
           (let ((entries (mapcar (lambda (name) (list (intern name) '((t :weight bold))))
                                  (scaling--names "scaling-pill" n))))
             (dolist (face '(romacs-mode-line-pill romacs-mode-line-state romacs-mode-line-state-pill))
               (put face 'theme-face entries))))
  :body (romacs-mode-line-pill-faces))
