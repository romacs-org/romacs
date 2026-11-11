;;; toasts-clear.el --- scaling probe  -*- lexical-binding: t -*-

;; Clearing N toasts (the echo area's clearing at the next command) with N
;; more layers kept hidden: each toast hidden once, no restacking per
;; toast, its layer found without a walk over the kept ones.
(scaling-probe toasts-clear :gui t :n 20 :steps 1
  :setup (progn (dotimes (i n) (romacs-layer--frame (get-buffer-create (format " scaling-layer-%d" i))))
                (setq romacs-layer-toast-max n romacs-layer-toast-timeout 600)
                (dotimes (i n) (romacs-layer-toast (format "toast %d" i)))
                n)
  :body (romacs-layer-toast-clear))
