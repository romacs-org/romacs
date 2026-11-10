;;; mode-line-bar-segments.el --- scaling probe  -*- lexical-binding: t -*-

;; The status bar's marks and paint (VM2-STATUS-BAR): N pill segments,
;; each with an icon state, its own pill colour and its own pill text
;; colour, laid out after each step's change.
(scaling-probe mode-line-bar-segments :gui t :n 40 :steps 20
  :setup (progn
           (require 'romacs-mode-line)
           (romacs-mode-line-mode 1)
           (dotimes (i n)
             (let ((face (intern (format "bar-probe-face-%d" i))))
               (make-face face)
               (set-face-attribute face nil :background (format "#%02x3040" (% (* 5 i) 256))
                                   :foreground (format "#e0%02xa0" (% (* 3 i) 256)))
               (eval `(romacs-mode-line-define-segment ,(intern (format "bar-probe-%d" i)) "A probe."
                        :pill ,face :pill-text ,face
                        :format (" " (:propertize "%*" romacs-icon ((?* . modified) (?- . saved))) " "))
                     t))
             (push (intern (format "bar-probe-%d" i)) romacs-mode-line-right))
           (romacs-mode-line-rebuild)
           (switch-to-buffer (scaling--lines-buffer "scaling-bar" 50)))
  :body (with-current-buffer it (insert "x")))
