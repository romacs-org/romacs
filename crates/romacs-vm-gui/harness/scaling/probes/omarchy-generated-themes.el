;;; omarchy-generated-themes.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe omarchy-generated-themes :n 40
  :setup (progn
           (require 'romacs-omarchy)
           (cons (mapcar #'intern (scaling--names "omarchy-scaling" n))
                 '((mode dark) (bg "#101418") (fg "#d8dee9") (red "#e06c75") (green "#98c379")
                   (yellow "#e5c07b") (blue "#61afef") (magenta "#c678dd") (cyan "#56b6c2"))))
  :body (let ((palette (cdr it)))
          (dolist (theme (car it)) (omarchy--define theme "scaling" palette))
          (omarchy--define (car (car it)) "scaling" palette)))
