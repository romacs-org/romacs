;;; world-layer-refusals.el --- scaling probe  -*- lexical-binding: t -*-

;; An init layer build's refusal index (world::layer `Deferred'): N
;; libraries whose configuration is refused, half by feature and half by
;; file, then N declared entries each asking which of them its loads
;; (three features, three nested files) hold, as the builder asks.
(scaling-probe world-layer-refusals :n 2000
  :setup (let ((names (scaling--names "scaling-refused" n)) (i 0) deferred loaded)
           (dolist (name names)
             (push (list name (and (cl-oddp i) (format "/scaling/lib/%s.el" name))) deferred)
             (push (list (list (format "%s-a" name) (format "%s-b" name) (if (cl-evenp i) name "none"))
                         (list (format "/scaling/x/%s-1.el" name) (format "/scaling/x/y/%s-2.el" name)
                               (format "/scaling/lib/%s.el" name)))
                   loaded)
             (setq i (1+ i)))
           (list deferred loaded))
  :body (romacs--world-layer-deferred-by (car it) (cadr it)))
