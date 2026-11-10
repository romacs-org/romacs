;;; fontset-long-group.el --- scaling probe  -*- lexical-binding: t -*-

;; N alternatives on a range whose first one has every character: a new
;; character reads the group in its order, not all its alternatives.
(scaling-probe fontset-long-group :gui t :n 200 :steps 10
  :setup (progn
           (set-fontset-font t '(#x400 . #x4ff) (font-spec :registry "iso10646-1"))
           (dotimes (_ n) (set-fontset-font t '(#x400 . #x4ff) "No Such Family" nil 'append)))
  :body (dotimes (c 12) (insert (+ #x410 (* i 12) c))))
