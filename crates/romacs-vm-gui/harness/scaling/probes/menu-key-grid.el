;;; menu-key-grid.el --- scaling probe  -*- lexical-binding: t -*-

;; A key grid of N entries: one tall group of N/2 and N/2 more in groups
;; of four, laid out in bands; linear, each row one `aref' per block.
(scaling-probe menu-key-grid :gui t :n 400 :steps 1
  :setup (let ((entry (lambda (i) (cons (format "k%d" i) (format "command %d" i)))))
           (cons (switch-to-buffer (get-buffer-create "scaling-grid"))
                 (cons (cons "Tall" (mapcar entry (number-sequence 1 (/ n 2))))
                       (seq-map-indexed (lambda (_ g) (cons (format "G%d" g) (mapcar entry (number-sequence 1 4))))
                                        (make-list (/ n 8) nil)))))
  :body (progn (romacs-popup-key-grid (cdr it) 160)
               (with-current-buffer (car it) (insert "x"))))
