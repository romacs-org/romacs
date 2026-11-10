;;; options-search.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe options-search :n 2000
  :setup (prog1 nil (scaling--declare-options "scaling-search" n) (romacs-option--current))
  :body (dotimes (_ 20) (romacs-option-search "scaling-search no-such-word")))
