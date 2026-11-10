;;; libxml-xml-prefixes.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe libxml-xml-prefixes :n 2000
  :setup (concat "<a "
                 (mapconcat (lambda (i) (format "xmlns:p%d='u%d'" i i)) (number-sequence 1 n) " ")
                 ">"
                 (mapconcat (lambda (i) (format "<p%d:b/>" i)) (number-sequence 1 n) "")
                 "</a>")
  :body (with-temp-buffer (insert it) (libxml-parse-xml-region)))
