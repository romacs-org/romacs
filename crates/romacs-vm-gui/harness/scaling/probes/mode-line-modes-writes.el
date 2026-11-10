;;; mode-line-modes-writes.el --- scaling probe  -*- lexical-binding: t -*-

;; One more package entry written to a `mode-line-modes' of N: the
;; watcher rebuilds the extras once per write, linear in the entries,
;; and nothing per key or redisplay.
(scaling-probe mode-line-modes-writes :gui t :n 50 :steps 10
  :setup (let ((saved (default-value 'mode-line-modes)))
           (dotimes (i n) (add-to-list 'mode-line-modes (list t (format "[p%d] " i)) t))
           (cons (switch-to-buffer (get-buffer-create "scaling-modes")) saved))
  :body (progn (setq-default mode-line-modes (cons (list t (format "[w%d] " i)) (cdr (default-value 'mode-line-modes))))
               (with-current-buffer (car it) (insert "x"))))
