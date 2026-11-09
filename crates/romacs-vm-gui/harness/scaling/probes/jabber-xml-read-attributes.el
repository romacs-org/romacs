;;; jabber-xml-read-attributes.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe jabber-xml-read-attributes :n 2000
  :setup (progn (scaling--jabber)
                (concat "<presence"
                        (mapconcat (lambda (i) (format " a%d='%d'" i i)) (number-sequence 1 n) "")
                        "/>"))
  :body (with-temp-buffer (insert it) (jabber-xml-parse-next-stanza)))
