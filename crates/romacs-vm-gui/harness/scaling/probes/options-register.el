;;; options-register.el --- scaling probe  -*- lexical-binding: t -*-

;; Declaring and adding marks the cache and no more, and one query rebuilds.
(scaling-probe options-register :n 500
  :setup (progn (require 'romacs-option) (mapcar #'intern (scaling--names "scaling-register" n)))
  :body (progn (dolist (option it)
                 (custom-declare-variable option 0 "A scaling option." :type 'integer :group 'scaling-options))
               (romacs-option-search "scaling-register no-such-word")
               (custom-add-to-group 'scaling-options 'scaling-register-extra 'custom-variable)
               (romacs-option-search "scaling-register no-such-word")))
