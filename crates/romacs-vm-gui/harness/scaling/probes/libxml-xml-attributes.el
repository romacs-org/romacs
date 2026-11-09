;;; libxml-xml-attributes.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe libxml-xml-attributes :n 4000
  :setup (concat "<a" (mapconcat (lambda (i) (format " a%d='%d'" i i)) (number-sequence 1 n) "") "/>")
  :body (with-temp-buffer (insert it) (libxml-parse-xml-region)))
