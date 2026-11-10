;;; mode-line-bar-pill-text.el --- scaling probe  -*- lexical-binding: t -*-

;; N pills on one pill face, each a different character in its own text
;; colour, all shown (the right side shows whole): the painter's
;; recoloured faces and their character bindings.
(scaling-probe mode-line-bar-pill-text :gui t :n 12 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (setq romacs-mode-line-right nil)
           (dotimes (i n)
             (let ((face (intern (format "bar-text-face-%d" i))))
               (make-face face)
               (set-face-attribute face nil :foreground (format "#e0%02x%02x" (% (* 7 i) 256) (% (* 11 i) 256)))
               (eval `(romacs-mode-line-define-segment ,(intern (format "bar-text-%d" i)) "A probe."
                        :pill romacs-mode-line-pill :pill-text ,face
                        :format ,(format " %c " (+ ?α i)))
                     t)
               (push (intern (format "bar-text-%d" i)) romacs-mode-line-right)))
           (romacs-mode-line-rebuild)
           (switch-to-buffer (scaling--lines-buffer "scaling-bar-text" 50)))
  :body (with-current-buffer it (insert "x")))
