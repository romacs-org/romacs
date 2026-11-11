;;; property-walk-aliases.el --- scaling probe  -*- lexical-binding: t -*-

;; A walk of N `face' runs by `next-single-property-change' while
;; `char-property-alias-alist' holds N unrelated entries: a run that has
;; the property itself asks nothing of the aliases, so the walk costs its
;; runs, not runs times entries.
(scaling-probe property-walk-aliases :n 2000
  :setup (with-current-buffer (get-buffer-create " scaling-property-walk")
           (erase-buffer)
           (insert (make-string (* 2 n) ?x))
           (dotimes (i n)
             (put-text-property (+ 1 (* 2 i)) (+ 2 (* 2 i)) 'face (if (cl-evenp i) 'bold 'italic))
             (put-text-property (+ 2 (* 2 i)) (+ 3 (* 2 i)) 'face (if (cl-evenp i) 'italic 'bold)))
           (cons (current-buffer)
                 (let (alist) (dotimes (i n) (push (list (intern (format "scaling-alias-%d" i)) 'scaling-other) alist)) alist)))
  :body (with-current-buffer (car it)
          (let ((char-property-alias-alist (cdr it)) (p (point-min)) (c 0))
            (while (setq p (next-single-property-change p 'face)) (setq c (1+ c)))
            c)))
