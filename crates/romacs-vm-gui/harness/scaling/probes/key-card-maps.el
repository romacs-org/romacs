;;; key-card-maps.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe key-card-maps :gui t :n 20 :steps 1
  :setup (progn
           (dotimes (i n)
             (let ((map (make-sparse-keymap)) (mode (intern (format "scaling-card-mode-%d" i))))
               (define-key map (vector 'f12 (+ #x1000 i)) #'forward-char)
               (set mode t)
               (push (cons mode map) minor-mode-map-alist)))
           (switch-to-buffer (get-buffer-create "scaling-card-maps")))
  :body (progn (romacs-popup--card-layout [f12]) (with-current-buffer it (insert "x"))))
