;;; popup-panels-cycle.el --- scaling probe  -*- lexical-binding: t -*-

;; The hint target after N panels shown and their buffers killed: a
;; killed panel leaves the registry, so the target asks about none.
(scaling-probe popup-panels-cycle :gui t :n 20 :steps 10
  :setup (progn (dotimes (i n)
                  (let ((buffer (get-buffer-create (format " scaling-panel-%d" i))))
                    (with-current-buffer buffer (insert "x"))
                    (romacs-popup-show 'panel buffer :lines 1)
                    (kill-buffer buffer)))
                (set-transient-map (define-keymap "a" #'ignore) t)
                (switch-to-buffer (get-buffer-create "scaling-panels")))
  :body (progn (romacs-popup--hint-target)
               (with-current-buffer it (insert "x"))))
