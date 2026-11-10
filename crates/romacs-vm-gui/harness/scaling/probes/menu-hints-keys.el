;;; menu-hints-keys.el --- scaling probe  -*- lexical-binding: t -*-

;; Hint entries for a map of N bindings whose sparse child rebinds or
;; unbinds half of them: one traversal, a seen-key hash, no lookup per key.
(scaling-probe menu-hints-keys :gui t :n 400 :steps 1
  :setup (let ((parent (make-sparse-keymap)) (child (make-sparse-keymap)))
           (dotimes (i n) (define-key parent (vector (+ #x1000 i)) #'ignore))
           (dotimes (i (/ n 2)) (define-key child (vector (+ #x1000 (* 2 i))) (and (cl-oddp i) #'forward-char)))
           (set-keymap-parent child parent)
           (cons (switch-to-buffer (get-buffer-create "scaling-hint-keys")) child))
  :body (progn (romacs-popup--hint-entries (cdr it))
               (with-current-buffer (car it) (insert "x"))))
