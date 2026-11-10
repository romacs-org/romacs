;;; jabber-xml-read-stanza.el --- scaling probe  -*- lexical-binding: t -*-

;; The stanza reader (jabber-xml.el, natively romacs--xml-read-element):
;; one stanza of N children, one of N attributes; and a connection's
;; buffer of N stanzas read and deleted one at a time, as jabber-filter
;; does, which stays linear because a read looks at a window of the
;; buffer, not all that follows.
(scaling-probe jabber-xml-read-stanza :n 2000
  :setup (progn (scaling--jabber)
                (concat "<message>"
                        (mapconcat (lambda (i) (format "<b x='%d'>t &amp; u</b>" i))
                                   (number-sequence 1 n) "")
                        "</message>"))
  :body (with-temp-buffer (insert it) (jabber-xml-parse-next-stanza)))
