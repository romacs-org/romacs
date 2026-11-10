;;; menu-hints-ranges.el --- scaling probe  -*- lexical-binding: t -*-

;; Hint entries for a child of N character ranges over a parent of 2N
;; keys inside and beside them and a grandparent of N ranges overlapping
;; them all: each interval painted once over the sorted bounds, never a
;; scan of the keys per range.
(scaling-probe menu-hints-ranges :gui t :n 200 :steps 1
  :setup (let ((grand (make-keymap)) (parent (make-sparse-keymap)) (child (make-keymap)))
           (dotimes (i n)
             (let ((lo (+ #x1000 (* 10 i))))
               (set-char-table-range (nth 1 child) (cons lo (+ lo 3)) (if (cl-oddp i) #'forward-char #'backward-char))
               (define-key parent (vector (+ lo 1)) #'ignore)
               (define-key parent (vector (+ lo 5)) #'ignore)
               (set-char-table-range (nth 1 grand) (cons (+ lo 2) (+ lo 7)) (if (cl-oddp i) #'undo #'redo))))
           (set-keymap-parent parent grand)
           (set-keymap-parent child parent)
           (cons (switch-to-buffer (get-buffer-create "scaling-hint-ranges")) child))
  :body (progn (romacs-popup--hint-entries (cdr it))
               (with-current-buffer (car it) (insert "x"))))
