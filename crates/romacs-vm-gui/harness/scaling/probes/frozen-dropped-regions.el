;;; frozen-dropped-regions.el --- scaling probe  -*- lexical-binding: t -*-

;; Dropped frozen values (frozen.rs, gc.rs `Vm::minor'): N values of a
;; page or more, each in a region with its memfd, frozen in a function and
;; dropped as it returns, as background fontification receives and thaws
;; its results, beside the young garbage that runs minors.  A minor frees
;; the regions attached since the last collection that nothing reaches, so
;; few are live at once however many were made (romacs#373: 264, until a
;; full collection).  The body counts them at each step, which costs what
;; is live: a leak is quadratic work, and past 64 at once it fails.  Each
;; step's garbage is 320 KB, so a minor (due at 4 MiB) comes every 13
;; steps or so.  (A value frozen by the loop's own forms, rather than in a
;; function, can still be held where the minor runs, and outlive it.)
(defun scaling--freeze-and-drop ()
  "Freeze a page-sized string and drop it."
  (freeze (make-string 5000 ?x))
  nil)
(scaling-probe frozen-dropped-regions :n 400
  :setup nil
  :body (let ((peak 0) (start gcs-done))
          (dotimes (_ n)
            (scaling--freeze-and-drop)
            (make-list 20000 nil)
            (setq peak (max peak (length (romacs--frozen-regions)))))
          (when (= gcs-done start) (error "No collection ran"))
          (when (> peak 64) (error "%d frozen regions live at once" peak))))
