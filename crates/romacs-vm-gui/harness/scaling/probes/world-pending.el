;;; world-pending.el --- scaling probe  -*- lexical-binding: t -*-

;; An init layer's pending definitions (world/pending.rs): N variables
;; made pending, then each read once, which evaluates its definition.
(scaling-probe world-pending :n 2000
  :setup (let ((symbols (mapcar #'intern (scaling--names "scaling-pending" n))))
           (dolist (symbol symbols) (set-default symbol 'stale))
           (setq romacs--world-pending
                 (vconcat (mapcar (lambda (symbol) (list 'defvar symbol (list 'defvar symbol 1) nil t)) symbols)))
           symbols)
  :body (let ((index 0))
          (dolist (symbol it)
            (romacs--world-pend symbol index)
            (setq index (1+ index)))
          (dolist (symbol it) (symbol-value symbol))))
