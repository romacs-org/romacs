;;; libxml-xml-attribute-entities.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe libxml-xml-attribute-entities :n 1000
  :setup (concat "<!DOCTYPE a ["
                 (mapconcat (lambda (i) (format "<!ENTITY e%d 'v'>" i)) (number-sequence 1 n) "")
                 "]><a "
                 (mapconcat (lambda (i) (format "b%d='&e%d;' c%d='&e%d;'" i i i i)) (number-sequence 1 n) " ")
                 "/>")
  :body (with-temp-buffer (insert it) (libxml-parse-xml-region)))
