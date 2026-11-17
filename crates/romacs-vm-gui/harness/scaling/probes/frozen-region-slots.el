;;; frozen-region-slots.el --- scaling probe  -*- lexical-binding: t -*-

;; Short list walks after N frozen regions were live at once and then
;; collected.  Every list walk (`length', `member', the reader, keymap
;; walks) is bounded by the conses the VM holds (`Vm::list_limit'), the
;; frozen regions' among them.  The region table keeps a slot for every
;; region ever live at once, swept or not, and the regions' conses were
;; once summed over every slot on each walk: a session's typing keys cost
;; more after each burst of frozen messages, and stayed so after the
;; regions were collected (romacs#283).  They are kept as regions come
;; and go, so a walk costs the same at any N.  Held by its slope,
;; instructions per region slot, not by an exponent over a fixed cost.
;; Measured 2026-11-17: 0.27, 0.0 and 0.0 now, 18,000 with the sum (each
;; of the 4,000 walks paid about 4.5 instructions a slot); a budget of 1
;; leaves the residue room and fails any per-slot walk.  K*N stays under
;; the 4,095 regions a VM can hold at once.
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
          (member 'z it)))
