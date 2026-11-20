;;; gc-minor-old-symbols.el --- scaling probe  -*- lexical-binding: t -*-

;; Minor collections (gc.rs `Vm::minor') with N old symbols of an obarray
;; of its own and N old conditions, which no minor frees.  What a minor
;; reads of them is what was made or put since the last collection
;; (`names_added', the conditions' log), so their number costs it nothing.
;; The body makes young garbage until eight minors have run.  Held by its
;; slope, instructions per old symbol and condition over the eight minors:
;; 10024 when a minor read every such name and property.  What is left,
;; 252, is none of that (the symbols alone cost under one instruction a
;; minor): the setup's garbage leaves free entries spread through the
;; tables, which the young fill first (`Table::alloc' takes the lowest),
;; so a minor's free-entry search and its sweep of the young's cards span
;; the old region, as in `gc-minor-old-heap'.  Compacting such tables
;; (romacs#307) is what removes it.
(defvar scaling--minor-obarray nil
  "The obarray the `gc-minor-old-symbols' probe keeps.")
(scaling-probe gc-minor-old-symbols :n 20000 :slope 300
  :setup (progn
           (setq scaling--minor-obarray (obarray-make))
           (dotimes (i n)
             (intern (format "scaling-old-%d" i) scaling--minor-obarray)
             (define-error (intern (format "scaling-old-error-%d" i)) "Old"))
           nil)
  :body (let ((until (+ gcs-done 8)))
          (while (< gcs-done until) (make-list 1000 nil))))
