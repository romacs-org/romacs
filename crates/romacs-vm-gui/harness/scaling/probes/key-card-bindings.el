;;; key-card-bindings.el --- scaling probe  -*- lexical-binding: t -*-

;; A key-hint card for a prefix of N bindings over M active minor-mode
;; maps that bind under it and R which-key replacements: one traversal of
;; each map, one replacement pass per entry, one layout; linear in each.
(scaling-probe key-card-bindings :gui t :n 100 :steps 1
  :setup (let ((map (make-sparse-keymap)))
           (dotimes (i n) (define-key map (vector 'f12 (+ #x1000 i)) #'forward-char))
           (use-local-map map)
           (switch-to-buffer (get-buffer-create "scaling-card")))
  :body (progn (romacs-popup--card-layout [f12]) (with-current-buffer it (insert "x"))))
