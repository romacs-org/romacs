;;; case-table-install.el --- scaling probe  -*- lexical-binding: t -*-

;; Installing a buffer's case table among N live strings: `set-case-table'
;; (and so `with-case-table', which gnosis's cloze rendering runs per card)
;; sets the current buffer's roots and touches no other buffer.  It once
;; walked every object in the heap first, for buffers with no roots of
;; their own, which only a change of the standard table needs: work in
;; the heap's size per call, where GNU's is constant.  Held by its slope,
;; instructions per live object, not by an exponent over a fixed cost.
;; Measured 2026-11-17: 0.01 now, 3,900 with the walk (each of the 300
;; installs touched every object, about 13 instructions apiece); a budget
;; of 1 leaves the residue two orders of room and fails any walk.
(defvar scaling--case-table-install nil)
(scaling-probe case-table-install :n 50000 :k 4 :slope 1
  :setup (progn
           (setq scaling--case-table-install (make-vector n nil))
           (dotimes (i n) (aset scaling--case-table-install i (make-string 8 ?x)))
           (generate-new-buffer " scaling-case-table-install"))
  :body (with-current-buffer it
          (let ((table (standard-case-table)))
            (dotimes (_ 100)
              (set-case-table table)
              (with-case-table table (upcase "x"))))))
