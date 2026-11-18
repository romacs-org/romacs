;;; gc-minor-old-heap.el --- scaling probe  -*- lexical-binding: t -*-

;; Minor collections (gc.rs `Vm::minor') over an old heap of N items of
;; every kind a table holds: a cons, a vector, a string, and a closure
;; over a mutable variable (a closure and a cell).  Three of every four
;; made are dropped, so the tables are mostly free entries, as a session's
;; are once something large has come and gone, and the young are handed
;; out among the old (`Table::alloc' takes the lowest free entry).  The
;; runner's full collection before the body makes the rest old.  The body
;; makes young garbage until eight minors have run: what a minor reads is
;; the young, what was stored into since the last collection and the
;; roots, so the old heap's size costs it nothing.  Held by its slope,
;; instructions per old item over the eight minors: a pass over every
;; table entry or colour shows here, however small its cost an entry.
(scaling-probe gc-minor-old-heap :n 50000 :slope 1
  :setup (let ((all nil))
           (dotimes (i (* 4 n))
             (let* ((x i) (item (list x (vector x x) (make-string 8 ?x) (lambda () (setq x (1+ x))))))
               (when (zerop (% i 4)) (push item all))))
           (setq scaling--live all)
           nil)
  :body (let ((until (+ gcs-done 8)))
          (while (< gcs-done until) (make-list 1000 nil))))
