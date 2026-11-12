;;; font-objects-opened.el --- scaling probe  -*- lexical-binding: t -*-

;; `font-info' for N fonts, each the frame's family at another pixel size,
;; then the same N again: the display keeps one font-object per font
;; opened (font.c font_open_entity's object list), indexed by font and
;; size, so the repeats are lookups and keeping one more is constant.
(scaling-probe font-objects-opened :gui t :n 100 :steps 1
  :setup (let ((family (face-attribute 'default :family)))
           (cons (switch-to-buffer (get-buffer-create "scaling-font-objects"))
                 (mapcar (lambda (i) (format "%s:pixelsize=%d" family (+ 6 i))) (number-sequence 0 (1- n)))))
  ;; The step paints (a character typed in the shown buffer), as a GUI
  ;; probe's must.
  :body (progn (dotimes (_ 2) (dolist (name (cdr it)) (font-info name)))
               (with-current-buffer (car it) (insert "x"))))
