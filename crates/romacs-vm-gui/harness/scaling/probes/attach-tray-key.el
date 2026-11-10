;;; attach-tray-key.el --- scaling probe  -*- lexical-binding: t -*-

;; A key typed into the draft under N chips: the cached tray string is
;; put again as it is, so the key's cost does not grow with N.
(scaling-probe attach-tray-key :gui t :n 100 :steps 20 :budget 0.5
  :setup (progn (require 'romacs-attach) (scaling--attach-host (scaling--attach-files n)))
  :body (with-current-buffer it (goto-char (point-max)) (insert "x")))
