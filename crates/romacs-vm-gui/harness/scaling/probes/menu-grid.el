;;; menu-grid.el --- scaling probe  -*- lexical-binding: t -*-

;; A key menu of N entries, one tall group of N/2 and N/2 more in groups
;; of four, laid out by `romacs-grid-pages' in a 30-row room: the paged
;; fill is linear in the blocks for each of the room's row budgets, and
;; each page's flow tries at most as many columns as the width holds.
(scaling-probe menu-grid :gui t :n 400 :steps 1
  :setup (let ((entry (lambda (i) (format "  k%d  command %d" i i))))
           (require 'romacs-grid)
           (cons (switch-to-buffer (get-buffer-create "scaling-grid"))
                 (cons (cons "Tall" (mapcar entry (number-sequence 1 (/ n 2))))
                       (seq-map-indexed (lambda (_ g) (cons (format "G%d" g) (mapcar entry (number-sequence 1 4))))
                                        (make-list (/ n 8) nil)))))
  :body (progn (romacs-grid-pages (cdr it) 30 160)
               (with-current-buffer (car it) (insert "x"))))
