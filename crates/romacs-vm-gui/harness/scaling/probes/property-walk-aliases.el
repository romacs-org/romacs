;;; property-walk-aliases.el --- scaling probe  -*- lexical-binding: t -*-

;; Walks of N property runs while `char-property-alias-alist' holds N
;; unrelated entries after the one that matters: `face' runs walked by
;; `next-single-property-change', which ask nothing of the aliases, and
;; `font-lock-face' runs asked for `face' through its alias, which stop at
;; its entry as GNU's `assq' does.  Each costs its runs, not runs times
;; entries.
(scaling-probe property-walk-aliases :n 2000
  :setup (with-current-buffer (get-buffer-create " scaling-property-walk")
           (erase-buffer)
           (insert (make-string (* 4 n) ?x))
           (dotimes (i n)
             (put-text-property (+ 1 (* 2 i)) (+ 2 (* 2 i)) 'face (if (cl-evenp i) 'bold 'italic))
             (put-text-property (+ 2 (* 2 i)) (+ 3 (* 2 i)) 'face (if (cl-evenp i) 'italic 'bold))
             (put-text-property (+ 1 (* 2 n) (* 2 i)) (+ 2 (* 2 n) (* 2 i)) 'font-lock-face (if (cl-evenp i) 'bold 'italic)))
           (cons (current-buffer)
                 (let (alist)
                   (dotimes (i n) (push (list (intern (format "scaling-alias-%d" i)) 'scaling-other) alist))
                   (cons '(face font-lock-face) alist))))
  :body (with-current-buffer (car it)
          (let ((char-property-alias-alist (cdr it)) (p (point-min)) (c 0))
            (while (setq p (next-single-property-change p 'face)) (setq c (1+ c)))
            (list c (text-property-any (1+ (* 2 n)) (point-max) 'face 'absent)))))
