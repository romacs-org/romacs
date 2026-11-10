;;; libxml-xml-declarations.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe libxml-xml-declarations :n 2000
  :setup (concat "<!DOCTYPE a ["
                 (mapconcat (lambda (i) (format "<!ELEMENT e%d (b)><!ATTLIST e%d c NMTOKEN #IMPLIED>" i i)) (number-sequence 1 n) "")
                 "]><a/>")
  :body (with-temp-buffer (insert it) (libxml-parse-xml-region)))
