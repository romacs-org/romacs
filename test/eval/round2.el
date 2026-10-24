;; -*- lexical-binding: t; -*-
(defun r2-call (f) (condition-case e (list 'ok (funcall f)) (error e)))
(let* ((s (intern "r2-fresh")) (k (intern ":r2-keyword")))
 (princ (prin1-to-string
  (list
   (r2-call (make-byte-code 0 (unibyte-string 8 135) (vector s) 1))
   (r2-call (make-byte-code 0 (unibyte-string 193 16 192 135) (vector k 1) 1))
   (r2-call (make-byte-code 0 (unibyte-string 193 24 41 192 135) (vector k 1) 1))
   (r2-call (make-byte-code 0 (unibyte-string 192 84 135) [1.5] 1))
   (r2-call (make-byte-code 0 (unibyte-string 192 32 135) [42] 1))
   (progn (let ((r2-fresh 1))) (boundp s))
   (r2-call (make-byte-code 0 (unibyte-string 192 84 135) [nil] 1))))))
(let* ((p (make-symbol ":r2-private"))
       (o (obarray-make))
       (s (intern ":r2-private" o))
       (k (intern ":r2-initial")))
  (prin1 (list (boundp p) (boundp s) (keywordp s) (keywordp k)
               (eq (symbol-value k) k) (1+ 1.5))))
