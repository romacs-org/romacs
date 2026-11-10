;;; fontset-edit-among-charsets.el --- scaling probe  -*- lexical-binding: t -*-

;; Edits among N charsets: the charset order is read when it changes.
(scaling-probe fontset-edit-among-charsets :n 40
  :setup (dotimes (i n)
           (define-charset (intern (format "scaling-charset-%d" i)) "A charset for a probe."
             :code-space [0 255] :code-offset #x110000))
  :body (dotimes (_ 200) (set-fontset-font t ?Ж "Y") (fontset-font t ?Ж)))
