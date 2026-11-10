;;; jabber-xml-read-buffer.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe jabber-xml-read-buffer :n 2000
  :setup (progn (scaling--jabber)
                (mapconcat (lambda (i) (format "<presence from='r@m/n%d'><x xmlns='u'/></presence>" i))
                           (number-sequence 1 n) ""))
  :body (with-temp-buffer
          (insert it)
          (while (jabber-xml-parse-next-stanza)
            (delete-region (point-min) (point)))))
