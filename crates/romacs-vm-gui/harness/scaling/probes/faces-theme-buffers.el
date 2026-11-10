;;; faces-theme-buffers.el --- scaling probe  -*- lexical-binding: t -*-

;; Nothing in a theme switch visits a buffer: text names faces.
(scaling-probe faces-theme-buffers :gui t :n 100 :steps 1 :budget 0.3
  :setup (progn
           (deftheme scaling-small "One face.")
           (custom-theme-set-faces 'scaling-small '(default ((t :background "#101010" :foreground "#e0e0e0"))))
           (dotimes (i n)
             (with-current-buffer (get-buffer-create (format "scaling-text-%d" i))
               (insert (propertize "some fontified text\n" 'face 'font-lock-keyword-face))))
           n)
  :body (progn (enable-theme 'scaling-small) (disable-theme 'scaling-small)))
