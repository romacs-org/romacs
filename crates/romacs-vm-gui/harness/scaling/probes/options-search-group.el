;;; options-search-group.el --- scaling probe  -*- lexical-binding: t -*-

;; A group's members are walked once: nothing asked of each option.
(scaling-probe options-search-group :n 2000
  :setup (prog1 nil (scaling--declare-options "scaling-group" n) (romacs-option--current))
  :body (dotimes (_ 20) (romacs-option-search "no-such-word" :group 'scaling-options)))
