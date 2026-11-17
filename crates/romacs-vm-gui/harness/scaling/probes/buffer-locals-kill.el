;;; buffer-locals-kill.el --- scaling probe  -*- lexical-binding: t -*-

;; Killing eight buffers with 50 locals each among N locals that 20 other
;; buffers hold: GNU's kill drops the buffer's own `local_var_alist' and touches no
;; other buffer's locals, so N costs it nothing.  The other buffers' count
;; stays fixed, as their list is `buffers''s probe; their locals grow.
;; Held by its slope, instructions per other buffer's local.  Measured
;; 2026-11-17: -0.05 with each buffer's locals in its own store, 172 when
;; a kill listed its bindings by walking every symbol's shared table; a
;; budget of 1 leaves the residue its noise and fails any such walk.
(scaling-probe buffer-locals-kill :n 2000 :k 4 :slope 1
  :setup (let ((syms (mapcar #'intern (scaling--names "scaling--kill-local" (/ n 20)))))
           (dotimes (b 20)
             (with-current-buffer (get-buffer-create (format " scaling-kill-other-%d" b))
               (dolist (s syms) (set (make-local-variable s) b))))
           (mapcar (lambda (v)
                     (with-current-buffer (get-buffer-create (format " scaling-kill-victim-%d" v))
                       (dotimes (i 50) (set (make-local-variable (nth i syms)) i))
                       (current-buffer)))
                   '(0 1 2 3 4 5 6 7)))
  :body (mapc #'kill-buffer it))
