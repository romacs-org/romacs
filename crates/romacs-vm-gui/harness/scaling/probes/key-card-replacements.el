;;; key-card-replacements.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe key-card-replacements :gui t :n 50 :steps 1
  :setup (progn
           (require 'which-key)
           (let ((map (make-sparse-keymap)))
             (dotimes (i 40) (define-key map (vector 'f12 (+ #x1000 i)) #'forward-char))
             (use-local-map map))
           (dotimes (i n) (which-key-add-key-based-replacements (format "<f11> %d" i) "scaling"))
           (switch-to-buffer (get-buffer-create "scaling-card-repl")))
  :body (progn (romacs-popup--card-layout [f12]) (with-current-buffer it (insert "x"))))
