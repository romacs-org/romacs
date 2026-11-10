;;; popup-panels-kept.el --- scaling probe  -*- lexical-binding: t -*-

;; One of N panels kept shown hidden and shown again: the panel registry
;; adds and removes it in O(1), the hint target stops at the first shown.
(scaling-probe popup-panels-kept :gui t :n 10 :steps 10
  :setup (let (buffers)
           (dotimes (i n)
             (let ((buffer (get-buffer-create (format " scaling-kept-panel-%d" i))))
               (with-current-buffer buffer (insert "x"))
               (romacs-popup-show 'panel buffer :lines 1)
               (push buffer buffers)))
           ;; An active map, so the hint target asks about the panels.
           (set-transient-map (define-keymap "a" #'ignore) t)
           (cons (switch-to-buffer (get-buffer-create "scaling-kept-panels")) (car (last buffers))))
  :body (progn (romacs-popup-hide (cdr it)) (romacs-popup-show 'panel (cdr it) :lines 1)
               (with-current-buffer (car it) (insert "x"))))
