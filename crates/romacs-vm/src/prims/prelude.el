;;; prelude.el --- VM2 T3 tier: loops that call Lisp  -*- lexical-binding: t -*-

;; A Rust primitive never calls Lisp from a loop (PRIMITIVE-DOCTRINE section 3).
;; A loop whose body calls Lisp is written here instead, as ordinary compiled
;; Lisp over leaf cursors (`car'/`cdr', `aref', `length'), the same shape as the
;; `maphash' expansion over `#:vm2-hash-next' in expand.rs.  Every value such a
;; loop keeps across a callback is then a register of a traced Lisp frame by
;; construction, and a collection inside the callback forwards it.

(defun mapcar (function sequence)
  "Apply FUNCTION to each element of SEQUENCE, and make a list of the results.
GNU fns.c `Fmapcar'/`mapcar1': the length is taken first, so an improper or
circular SEQUENCE signals before FUNCTION runs, and a list that FUNCTION
shortens ends the walk early."
  (let ((n (length sequence))
        (i 0)
        (acc nil))
    (if (or (consp sequence) (null sequence))
        (let ((tail sequence))
          (while (and (< i n) (consp tail))
            (setq acc (cons (funcall function (car tail)) acc))
            (setq tail (cdr tail))
            (setq i (1+ i))))
      (while (< i n)
        (setq acc (cons (funcall function (aref sequence i)) acc))
        (setq i (1+ i))))
    (nreverse acc)))

;;; prelude.el ends here
