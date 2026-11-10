;;; libxml-xml-entities.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe libxml-xml-entities :n 2000
  :setup (concat "<!DOCTYPE a ["
                 (mapconcat (lambda (i) (format "<!ENTITY e%d 'v'>" i)) (number-sequence 1 n) "")
                 "]><a>" (mapconcat (lambda (i) (format "&e%d;" i)) (number-sequence 1 n) "") "</a>")
  :body (with-temp-buffer (insert it) (libxml-parse-xml-region)))
