;;; fontset-edit-among-rules.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe fontset-edit-among-rules :n 2000
  :setup (dotimes (i n) (set-fontset-font t (+ #x10000 (* 2 i)) "X"))
  :body (dotimes (_ 200) (set-fontset-font t ?Ж "Y") (fontset-font t ?Ж)))
