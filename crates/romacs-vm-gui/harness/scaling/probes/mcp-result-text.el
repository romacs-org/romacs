;;; mcp-result-text.el --- scaling probe  -*- lexical-binding: t -*-

;; A tool result of N characters, Unicode text: its check is native, so
;; one character costs no Lisp call.
(scaling-probe mcp-result-text :n 20000
  :setup (progn (require 'romacs-mcp) (make-string n ?é))
  :body (dotimes (_ 20) (romacs-mcp--result it nil t)))
