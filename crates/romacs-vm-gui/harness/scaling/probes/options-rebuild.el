;;; options-rebuild.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe options-rebuild :n 2000
  :setup (scaling--declare-options "scaling-rebuild" n)
  :body (progn (romacs-option--stale) (romacs-option--current)))
