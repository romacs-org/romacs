;;; libxml-xml-children.el --- scaling probe  -*- lexical-binding: t -*-

;; `libxml-parse-xml-region' (crate::xml_read's document reader): N sibling
;; elements, N attributes on one, N declared entities each referred to, and
;; a run of N comments before the root: each stays linear in the document.
(scaling-probe libxml-xml-children :n 4000
  :setup (concat "<a>" (mapconcat (lambda (i) (format "<b c='%d'>t &amp; u</b>\n" i))
                                  (number-sequence 1 n) "")
                 "</a>")
  :body (with-temp-buffer (insert it) (libxml-parse-xml-region)))
