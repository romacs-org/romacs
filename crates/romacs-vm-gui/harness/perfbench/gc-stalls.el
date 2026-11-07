;;; gc-stalls.el --- The stalls a running program sees  -*- lexical-binding: t -*-
;; Gaps between loop iterations while it allocates, in any Emacs, byte-compiled
;; for GNU: TAG=name emacs -Q --batch -l gc-stalls.elc
(defun gap-live (n) (let (l) (dotimes (i n) (push (cons i (make-string 16 ?x)) l)) l))
(defun gap-churn (n)
  (let ((last (float-time)) (gaps nil) (t0 (float-time)))
    (dotimes (i n)
      (let ((x (make-vector 8 i)) (s (format "%d" i)) (c (list i i i))) (ignore x s c))
      (when (= 0 (% i 16))
        (let ((now (float-time)))
          (when (> (- now last) 0.002) (push (- now last) gaps))
          (setq last now))))
    (let* ((p (sort gaps #'<)) (k (length p)))
      (princ (format "   wall %5.2f s  stalls over 2 ms %4d  median %7.2f ms  max %7.2f ms  total %6.2f s\n"
                     (- (float-time) t0) k (* 1000 (if p (nth (/ k 2) p) 0)) (* 1000 (if p (car (last p)) 0)) (apply #'+ p))))))
(defun gap-run (tag live)
  (let ((keep (gap-live live))) (garbage-collect) (princ (concat tag "\n")) (gap-churn 6000000) (length keep)))
(gap-run (concat (getenv "TAG") " small heap") 10000)
(gap-run (concat (getenv "TAG") " 2M live objects") 2000000)
