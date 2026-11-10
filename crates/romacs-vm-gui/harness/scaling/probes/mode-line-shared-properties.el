;;; mode-line-shared-properties.el --- scaling probe  -*- lexical-binding: t -*-

;; `format-mode-line' of N pieces under one `:propertize' whose shared
;; keymap has N bindings: the map is made once for the string, not once
;; per piece (VM2-G1B).
(scaling-probe mode-line-shared-properties :n 200
  :setup (let ((map (list 'keymap)))
           (dotimes (i n) (push (cons (+ 1000 i) 'ignore) (cdr map)))
           (list :propertize (let (pieces) (dotimes (i n) (push (propertize "a" 'help-echo (number-to-string i)) pieces)) pieces)
                 'local-map map))
  :body (let* ((noninteractive nil) (s (format-mode-line it)))
          (unless (get-text-property 0 'local-map s) (error "The shared keymap is not projected"))
          s))
