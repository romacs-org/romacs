;;; frozen-region-slots.el --- scaling probe  -*- lexical-binding: t -*-

;; A key's work after N frozen regions were live at once and then
;; collected: short list walks, and the collector's pacing.  The region
;; table keeps a slot for every region ever live at once, swept or not,
;; and what the regions add up to was once summed over every slot where
;; it is read: their conses on every list walk (`Vm::list_limit' bounds
;; `length', `member', the reader, keymap walks) and their weight wherever
;; the next collection's budget is read (`Vm::collection_budget': at every
;; poll, once per 256 KB allocated, and in `garbage-collect-maybe', which
;; reads it here without allocating).  A session's typing keys cost more
;; after each burst of frozen messages, and stayed so after the regions
;; were collected (romacs#283).  Both are kept as regions come and go, so
;; the work costs the same at any N.  Held by its slope, instructions per
;; region slot, not by an exponent over a fixed cost.  Measured
;; 2026-11-17: 0.0 three times with both kept; 9,000 with the conses kept
;; and the weight summed; with the walks alone, 18,000 with the conses
;; summed (about 4.5 instructions a slot a read).  A budget of 1 leaves
;; the residue room and fails any per-slot sum on either path.  K*N
;; stays under the 4,095 regions a VM can hold at once.
(defvar scaling--frozen-region-slots nil)
(scaling-probe frozen-region-slots :n 900 :k 4 :slope 1
  :setup (progn
           (setq scaling--frozen-region-slots (make-vector n nil))
           (dotimes (i n) (aset scaling--frozen-region-slots i (freeze (list i))))
           ;; Released here, swept by the runner's collection before the
           ;; body: their slots stay.
           (setq scaling--frozen-region-slots nil)
           (list 'a 'b 'c 'd))
  :body (dotimes (_ 2000)
          (length it)
          (member 'z it)
          (garbage-collect-maybe 0)))
