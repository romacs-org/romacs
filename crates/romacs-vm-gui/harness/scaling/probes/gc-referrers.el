;;; gc-referrers.el --- scaling probe  -*- lexical-binding: t -*-

;; Edits after a collection, among N live strings: each deletion over a
;; buffer's marker reads that buffer's markers from the referrers index
;; (`Vm::buffer_referrers'), for the undo list's marker adjustments.  The
;; runner collects before the body.  A collection that dropped the index
;; has the first edit list it again by a walk of the whole heap, about 30
;; instructions per live object; one that keeps it, forgetting what died
;; and following what moved, leaves the edits the same at any N.  Held by
;; its slope, instructions per live object, not by an exponent over a
;; fixed cost.  What slope is left is the tables': the first allocation
;; after a sweep looks for the lowest free entry above every live one
;; (`Table::alloc').
(defvar scaling--gc-referrers nil)
(scaling-probe gc-referrers :n 50000 :k 4 :slope 1
  :setup (progn
           (setq scaling--gc-referrers (make-vector n nil))
           (dotimes (i n) (aset scaling--gc-referrers i (make-string 8 ?x)))
           (with-current-buffer (scaling--lines-buffer "scaling-gc-referrers" 20)
             (put 'scaling--gc-referrers 'marker (copy-marker 30))
             ;; Listed before the runner's collection, as a session's
             ;; first edit over a marker lists it.
             (delete-region 29 31)
             (current-buffer)))
  :body (with-current-buffer it
          (dotimes (_ 100) (goto-char 25) (delete-char 10) (insert "abcdefghij"))))
