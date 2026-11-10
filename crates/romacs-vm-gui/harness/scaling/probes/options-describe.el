;;; options-describe.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe options-describe :n 2000
  :setup (prog1 (scaling--declare-options "scaling-describe" n) (romacs-option--current))
  :body (dolist (option it) (romacs-option-describe option)))
