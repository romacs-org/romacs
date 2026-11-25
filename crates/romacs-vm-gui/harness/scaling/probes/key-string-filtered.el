;;; key-string-filtered.el --- scaling probe  -*- lexical-binding: t -*-

;; A multibyte string key of N characters looked up in a keymap that
;; binds its character to a menu item whose filter gives the keymap
;; itself back: a filter runs at each of the N events.  After each filter
;; the walk finds its place in the string through the string's own
;; character/byte position, as GNU's char-to-byte cache does, not by
;; counting the characters already read: the lookup stays linear.  Each
;; filter call is a Lisp call of some 270k instructions, which hides a
;; re-count of the characters read at these sizes under the default
;; budget: linear work measured 1.000 here, the re-count 1.138, so the
;; budget is 1.05.
(scaling-probe key-string-filtered :n 1000 :budget 1.05
  :setup (let ((map (make-sparse-keymap)))
           (define-key map (string ?\xe9) (list 'menu-item "prefix" map :filter #'identity))
           (cons map (make-string n ?\xe9)))
  :body (lookup-key (car it) (cdr it)))
