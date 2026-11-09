;;; fontset-disjoint-rules.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe fontset-disjoint-rules :n 2000
  :setup nil
  :body (dotimes (i n) (set-fontset-font t (+ #x10000 (* 2 i)) "X")))
