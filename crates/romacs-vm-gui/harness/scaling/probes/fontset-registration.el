;;; fontset-registration.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe fontset-registration :n 1000
  :setup nil
  :body (dotimes (_ n) (fontset-font (new-fontset (scaling--fontset-name) nil) ?a)))
