;;; face-remap-bands.el --- scaling probe  -*- lexical-binding: t -*-

;; A buffer whose `face-remapping-alist' remaps N faces, shown in the
;; status bar and its window, redisplayed after each key in it: its
;; remapped face table is realized once and found again per redisplay
;; (`display_faces_for'), for the window's text and bands and for the
;; bar's cache key (VM2-FACE-REMAP).
(scaling-probe face-remap-bands :gui t :n 32 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (let ((buffer (scaling--lines-buffer "scaling-remap" 50)))
             (switch-to-buffer buffer)
             (setq header-line-format "remapped")
             (setq-local face-remapping-alist
                         (cl-loop for i below n
                                  collect (let ((face (intern (format "remap-probe-face-%d" i))))
                                            (make-face face)
                                            (list face (list :foreground (format "#%02x6080" (% (* 7 i) 256)))))))
             (push '(mode-line-active (:background "#203040")) face-remapping-alist)
             buffer))
  :body (with-current-buffer it (insert "x")))
