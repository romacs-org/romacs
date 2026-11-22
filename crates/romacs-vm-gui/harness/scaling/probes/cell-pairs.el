;;; cell-pairs.el --- scaling probe  -*- lexical-binding: t -*-

;; N closures made over environments of their own, as nadvice's and every
;; `add-function''s are, each by `make-interpreted-closure' from a fresh
;; lexical pair, and all kept: a pair's cell is found by an index
;; (cell_pairs.rs), so each closure costs the same however many pairs were
;; made before it.  Measured 2026-11-21: exponent 1.00; 1.80 when each
;; closure searched every pair made since the last collection (romacs#367).
(defvar scaling--cell-pairs nil "The closures the `cell-pairs' probe keeps.")
(scaling-probe cell-pairs :n 4000
  :setup (setq scaling--cell-pairs nil)
  :body (dotimes (i n)
          (push (make-interpreted-closure nil '(x) (list (cons 'x i) t) nil nil) scaling--cell-pairs)))
