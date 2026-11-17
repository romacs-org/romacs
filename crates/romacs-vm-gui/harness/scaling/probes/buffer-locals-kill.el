;;; buffer-locals-kill.el --- scaling probe  -*- lexical-binding: t -*-

;; Killing a buffer with 50 locals among N locals that 20 other buffers
;; hold: GNU's kill drops the buffer's own `local_var_alist' and touches no
;; other buffer's locals, so N costs it nothing.  The other buffers' count
;; stays fixed, as their list is `buffers''s probe; their locals grow.
;; Held by its slope, instructions per other buffer's local.
(scaling-probe buffer-locals-kill :n 2000 :k 4 :slope 1
  :known "romacs#59: each symbol keeps a binding table every buffer shares, and a kill lists the killed buffer's bindings by walking all of them"
  :setup (let ((syms (mapcar #'intern (scaling--names "scaling--kill-local" (/ n 20)))))
           (dotimes (b 20)
             (with-current-buffer (get-buffer-create (format " scaling-kill-other-%d" b))
               (dolist (s syms) (set (make-local-variable s) b))))
           (with-current-buffer (get-buffer-create " scaling-kill-victim")
             (dotimes (i 50) (set (make-local-variable (nth i syms)) i))
             (current-buffer)))
  :body (kill-buffer it))
