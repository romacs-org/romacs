;;; core-rows.el -*- lexical-binding: t; -*-
(defvar core-bench--all nil)
(defmacro core-bench--def (name &rest body)
  "Define bench row NAME running BODY."
  (declare (indent 1))
  (let ((function (intern (format "core-bench-row-%s" name))))
    `(progn
       (defun ,function () ,@body)
       (push (cons ',name ',function) core-bench--all))))

(defun core-bench--fib (n)
  (if (< n 2) n (+ (core-bench--fib (- n 1)) (core-bench--fib (- n 2)))))

(core-bench--def bc-fib-recursive
  (core-bench--fib 27))

(defun core-bench--loop (n)
  (let ((i 0) (s 0)) (while (< i n) (setq s (+ s i) i (1+ i))) s))

(core-bench--def bc-while-loop
  (core-bench--loop 20000000))

(defvar core-bench--special 0)

(defun core-bench--dyn (n)
  (let ((i 0)) (while (< i n) (let ((core-bench--special i)) (setq i (1+ i)))) i))

(core-bench--def bc-specbind-loop
  (core-bench--dyn 5000000))

(defun core-bench--funcall (n)
  (let ((f (lambda (x) (1+ x))) (i 0)) (while (< i n) (setq i (funcall f i))) i))

(core-bench--def bc-funcall-closure
  (core-bench--funcall 10000000))

(core-bench--def bc-mapcar-list
  (let ((l (number-sequence 1 1000)) last)
    (dotimes (_ 2000) (setq last (mapcar #'1+ l))) last))
