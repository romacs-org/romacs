;;; libxml-xml-parameter-entities.el --- scaling probe  -*- lexical-binding: t -*-

;; The collections of a document's internal subset, and its prefixes: N
;; parameter entities each expanded, N element and N attribute declarations,
;; N attribute entities each used twice, and N prefixes bound and used on
;; one element.
(scaling-probe libxml-xml-parameter-entities :n 1000
  :setup (concat "<!DOCTYPE a ["
                 (mapconcat (lambda (i) (format "<!ENTITY %% p%d '<!-- c -->'>%%p%d;" i i)) (number-sequence 1 n) "")
                 "]><a/>")
  :body (with-temp-buffer (insert it) (libxml-parse-xml-region)))
