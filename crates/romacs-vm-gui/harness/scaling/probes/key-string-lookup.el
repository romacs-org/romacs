;;; key-string-lookup.el --- scaling probe  -*- lexical-binding: t -*-

;; A key sequence that is a multibyte string of N characters, looked up in
;; a keymap that binds its character to itself, and defined in a fresh
;; keymap.  The keymap walk reads each event of a string from the byte
;; after the one it read last, so a lookup and a definition read the string
;; once, not from its start for each event.
(scaling-probe key-string-lookup :n 1000
  :setup (let ((map (make-sparse-keymap)))
           (define-key map (string ?\xe9) map)
           (cons map (make-string n ?\xe9)))
  :body (progn
          (lookup-key (car it) (cdr it))
          (define-key (make-sparse-keymap) (cdr it) #'ignore)))
