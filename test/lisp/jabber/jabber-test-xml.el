;;; jabber-test-xml.el --- Tests for jabber-xml  -*- lexical-binding: t; -*-

;;; Commentary:

;; XML serialization and node accessors.

;;; Code:

(require 'ert)
(require 'cl-lib)
(require 'jabber-xml)
(require 'jabber-core)

;;; Group 1: jabber-escape-xml / jabber-unescape-xml

(ert-deftest jabber-test-xml-escape-xml-special-chars ()
  "Escape ampersand, angle brackets, quotes and apostrophes."
  (should (string= (jabber-escape-xml "&<>\"'")
                   "&amp;&lt;&gt;&quot;&apos;")))

(ert-deftest jabber-test-xml-escape-xml-plain-string ()
  "Plain strings pass through unchanged."
  (should (string= (jabber-escape-xml "hello world") "hello world")))

(ert-deftest jabber-test-xml-escape-xml-nil ()
  "Non-string input is returned as-is."
  (should (eq (jabber-escape-xml nil) nil)))

(ert-deftest jabber-test-xml-escape-xml-control-chars ()
  "Control characters are replaced with spaces."
  (should (string= (jabber-escape-xml "\x01\x02") "  ")))

(ert-deftest jabber-test-xml-escape-xml-form-feed ()
  "Form feed is replaced with newline."
  (should (string= (jabber-escape-xml "a\fb") "a\nb")))

(ert-deftest jabber-test-xml-unescape-xml-entities ()
  "Unescape XML entities back to characters."
  (should (string= (jabber-unescape-xml "&amp;&lt;&gt;&quot;&apos;")
                   "&<>\"'")))

(ert-deftest jabber-test-xml-unescape-xml-nil ()
  "Non-string input is returned as-is."
  (should (eq (jabber-unescape-xml nil) nil)))

(ert-deftest jabber-test-xml-escape-unescape-roundtrip ()
  "Escaping then unescaping produces the original string."
  (let ((input "Tom & Jerry <friends> say \"hi\" & 'bye'"))
    (should (string= (jabber-unescape-xml (jabber-escape-xml input))
                     input))))

;;; Group 2: jabber-sexp2xml

(ert-deftest jabber-test-xml-sexp2xml-self-closing ()
  "Self-closing tag with no children."
  (should (string= (jabber-sexp2xml '(br nil))
                   "<br/>")))

(ert-deftest jabber-test-xml-sexp2xml-with-attributes ()
  "Tag with attributes and no children."
  (should (string= (jabber-sexp2xml '(stream ((to . "example.com") (xmlns . "jabber:client"))))
                   "<stream to='example.com' xmlns='jabber:client'/>")))

(ert-deftest jabber-test-xml-sexp2xml-with-children ()
  "Tag with text children."
  (should (string= (jabber-sexp2xml '(body nil "Hello"))
                   "<body>Hello</body>")))

(ert-deftest jabber-test-xml-sexp2xml-nested ()
  "Nested tags."
  (should (string= (jabber-sexp2xml '(message ((to . "bob@example.com"))
                                      (body nil "Hi")))
                   "<message to='bob@example.com'><body>Hi</body></message>")))

(ert-deftest jabber-test-xml-sexp2xml-string-input ()
  "String input is XML-escaped."
  (should (string= (jabber-sexp2xml "a<b") "a&lt;b")))

(ert-deftest jabber-test-xml-sexp2xml-empty-string-children ()
  "Work around old xml.el bug where (\"\") appears as children."
  (should (string= (jabber-sexp2xml '("")) "")))

(ert-deftest jabber-test-xml-sexp2xml-escapes-attribute-values ()
  "Attribute values are XML-escaped."
  (should (string= (jabber-sexp2xml '(a ((href . "a&b"))))
                   "<a href='a&amp;b'/>")))

;;; Group 3: node accessors

(ert-deftest jabber-test-xml-xml-node-name ()
  "Extract tag name from node."
  (should (eq (jabber-xml-node-name '(message ((type . "chat")) "hi"))
              'message)))

(ert-deftest jabber-test-xml-xml-node-name-nil ()
  "Non-list input returns nil."
  (should (eq (jabber-xml-node-name "string") nil)))

(ert-deftest jabber-test-xml-xml-node-attributes ()
  "Extract attributes alist from node."
  (should (equal (jabber-xml-node-attributes '(iq ((type . "get") (id . "1"))))
                 '((type . "get") (id . "1")))))

(ert-deftest jabber-test-xml-xml-node-children ()
  "Extract children from node."
  (should (equal (jabber-xml-node-children '(body nil "Hello"))
                 '("Hello"))))

(ert-deftest jabber-test-xml-xml-node-children-empty-string-bug ()
  "Work around old xml.el bug where children are ((\"\"))."
  (should (null (jabber-xml-node-children '(tag nil (""))))))

;;; Group 4: jabber-xml-get-children

(ert-deftest jabber-test-xml-xml-get-children-found ()
  "Find children by tag name."
  (let ((node '(iq nil (query nil "data") (error nil "oops"))))
    (should (equal (jabber-xml-get-children node 'query)
                   '((query nil "data"))))))

(ert-deftest jabber-test-xml-xml-get-children-missing ()
  "Return nil when tag not found."
  (let ((node '(iq nil (query nil))))
    (should (null (jabber-xml-get-children node 'error)))))

(ert-deftest jabber-test-xml-xml-get-children-multiple ()
  "Return all children with matching tag."
  (let ((node '(parent nil (item nil "a") (item nil "b"))))
    (should (= (length (jabber-xml-get-children node 'item)) 2))))

;;; Group 5: jabber-xml-get-attribute / jabber-xml-get-xmlns

(ert-deftest jabber-test-xml-xml-get-attribute-present ()
  "Get attribute value when present."
  (should (string= (jabber-xml-get-attribute '(iq ((type . "result"))) 'type)
                   "result")))

(ert-deftest jabber-test-xml-xml-get-attribute-missing ()
  "Return nil when attribute not found."
  (should (null (jabber-xml-get-attribute '(iq ((type . "result"))) 'id))))

(ert-deftest jabber-test-xml-xml-get-attribute-nil-node ()
  "Return nil for non-cons node."
  (should (null (jabber-xml-get-attribute nil 'type))))

(ert-deftest jabber-test-xml-xml-get-xmlns-present ()
  "Get xmlns attribute."
  (should (string= (jabber-xml-get-xmlns '(query ((xmlns . "jabber:iq:roster"))))
                   "jabber:iq:roster")))

(ert-deftest jabber-test-xml-xml-get-xmlns-absent ()
  "Return nil when xmlns not present."
  (should (null (jabber-xml-get-xmlns '(query nil)))))

;;; Group 6: jabber-xml-child-with-xmlns

(ert-deftest jabber-test-xml-xml-child-with-xmlns-found ()
  "Find child element by xmlns."
  (let ((node '(message nil
                 (x ((xmlns . "jabber:x:oob")) (url () "http://example.com"))
                 (body () "text"))))
    (should (equal (jabber-xml-node-name
                    (jabber-xml-child-with-xmlns node "jabber:x:oob"))
                   'x))))

(ert-deftest jabber-test-xml-xml-child-with-xmlns-missing ()
  "Return nil when no child has the given xmlns."
  (let ((node '(message nil (body () "text"))))
    (should-not (jabber-xml-child-with-xmlns node "jabber:x:oob"))))

(ert-deftest jabber-test-xml-xml-child-with-xmlns-nil-node ()
  "Return nil for nil node."
  (should-not (jabber-xml-child-with-xmlns nil "jabber:x:oob")))

(ert-deftest jabber-test-xml-xml-child-with-xmlns-skips-strings ()
  "String children are skipped without error."
  (let ((node '(body nil "just text")))
    (should-not (jabber-xml-child-with-xmlns node "some:ns"))))

;;; Group 7: jabber-xml-path


(ert-deftest jabber-test-xml-xml-path-symbol ()
  "Navigate to child by symbol name."
  (let ((xml '(iq nil (query nil "data"))))
    (should (equal (jabber-xml-path xml '(query))
                   '(query nil "data")))))

(ert-deftest jabber-test-xml-xml-path-string ()
  "String step extracts character data."
  (let ((xml '(body nil "Hello")))
    (should (string= (jabber-xml-path xml '(""))
                     "Hello"))))

(ert-deftest jabber-test-xml-xml-path-multi-step ()
  "Multi-step path navigation."
  (let ((xml '(iq nil (query nil (item ((jid . "bob@example.com")))))))
    (should (equal (jabber-xml-node-name
                    (jabber-xml-path xml '(query item)))
                   'item))))

(ert-deftest jabber-test-xml-xml-path-cons-namespace ()
  "Navigate using cons (namespace . name) step."
  (let ((xml '(message nil
                (delay ((xmlns . "urn:xmpp:delay") (stamp . "2024-01-01T00:00:00Z"))))))
    (should (equal (jabber-xml-node-name
                    (jabber-xml-path xml '(("urn:xmpp:delay" . "delay"))))
                   'delay))))

(ert-deftest jabber-test-xml-xml-path-missing ()
  "Return nil for non-existent path."
  (let ((xml '(iq nil (query nil))))
    (should (null (jabber-xml-path xml '(error))))))

;;; Group 8: jabber-xml-skip-tag-forward

(ert-deftest jabber-test-xml-skip-tag-forward-self-closing ()
  "Skip past a self-closing tag."
  (with-temp-buffer
    (insert "<br/>")
    (goto-char (point-min))
    (should (eq (catch 'unfinished
                  (jabber-xml-skip-tag-forward))
                t))
    (should (= (point) (point-max)))))

(ert-deftest jabber-test-xml-skip-tag-forward-paired ()
  "Skip past a paired open/close tag."
  (with-temp-buffer
    (insert "<body>Hello</body>")
    (goto-char (point-min))
    (should (eq (catch 'unfinished
                  (jabber-xml-skip-tag-forward))
                t))
    (should (= (point) (point-max)))))

(ert-deftest jabber-test-xml-skip-tag-forward-nested ()
  "Skip past nested tags."
  (with-temp-buffer
    (insert "<message><body>Hi</body></message>")
    (goto-char (point-min))
    (should (eq (catch 'unfinished
                  (jabber-xml-skip-tag-forward))
                t))
    (should (= (point) (point-max)))))

(ert-deftest jabber-test-xml-skip-tag-forward-cdata ()
  "Skip past CDATA section."
  (with-temp-buffer
    (insert "<![CDATA[some <data>]]>")
    (goto-char (point-min))
    (should (catch 'unfinished
              (jabber-xml-skip-tag-forward)))
    (should (= (point) (point-max)))))

(ert-deftest jabber-test-xml-skip-tag-forward-incomplete ()
  "Throw unfinished for incomplete tag."
  (with-temp-buffer
    (insert "<message><body>Hi</body>")
    (goto-char (point-min))
    (should-not (catch 'unfinished
                  (jabber-xml-skip-tag-forward)))))

;;; Group 9: jabber-xml-parse-next-stanza

(ert-deftest jabber-test-xml-parse-next-stanza-complete ()
  "Parse a complete XML stanza."
  (with-temp-buffer
    (insert "<message to='bob@example.com'><body>Hi</body></message>")
    (let ((result (jabber-xml-parse-next-stanza)))
      (should result)
      (should (eq (jabber-xml-node-name (car result)) 'message)))))

(ert-deftest jabber-test-xml-parse-next-stanza-incomplete ()
  "Return nil for incomplete stanza."
  (with-temp-buffer
    (insert "<message><body>Hi</body>")
    (should (null (jabber-xml-parse-next-stanza)))))

(ert-deftest jabber-test-xml-parse-next-stanza-empty ()
  "Return nil for empty buffer."
  (with-temp-buffer
    (should (null (jabber-xml-parse-next-stanza)))))

(ert-deftest jabber-test-xml-end-tag-whitespace-stream ()
  "Consume legal closing-tag whitespace, whole and fragmented, in order."
  (dolist (space '(" " "\t" "\n" "\r\n"))
    (dolist (fragmented '(nil t))
      (let* ((jc (make-symbol "xml-stream"))
             (buffer (generate-new-buffer " *jabber-test-xml*"))
             (proc (make-pipe-process :name "jabber-test-xml"
                                      :buffer buffer :noquery t))
             (prefix (concat "<message><body>hello</body" space))
             (suffix (concat "></message" space "><presence/>"))
             events)
        (unwind-protect
            (progn
              (put jc :state-data (list :connection proc))
              (cl-letf (((symbol-function 'fsm-send)
                         (lambda (_jc event) (push event events))))
                (if fragmented
                    (progn
                      (jabber-pre-filter proc prefix jc)
                      (should-not events)
                      (should (equal (with-current-buffer buffer (buffer-string))
                                     prefix))
                      (jabber-pre-filter proc suffix jc))
                  (jabber-pre-filter proc (concat prefix suffix) jc)))
              (should (equal (nreverse events)
                             '((:stanza (message nil (body nil "hello")))
                               (:stanza (presence nil)))))
              (should (zerop (buffer-size buffer))))
          (delete-process proc)
          (kill-buffer buffer))))))

(ert-deftest jabber-test-xml-end-tag-mismatch-not-consumed ()
  "Do not accept a different closing name as a complete stanza."
  (with-temp-buffer
    (insert "<message><body>hello</bodyx ></message><presence/>")
    (should-not (jabber-xml-parse-next-stanza))))

;;; Group 10: the native reader reads as xml.el does

(defun jabber-test-xml--by-xml-el (text)
  "TEXT's first stanza as xml.el reads it: what the native reader must match."
  (with-temp-buffer
    (insert text)
    (and (catch 'unfinished
           (goto-char (point-min))
           (jabber-xml-skip-tag-forward)
           (> (point) (point-min)))
         (cons (car (xml-parse-region (point-min) (point))) (point)))))

(defun jabber-test-xml--natively (text)
  "TEXT's first stanza as `romacs--xml-read-element' reads it, or `declined'."
  (with-temp-buffer
    (insert text)
    (pcase (romacs--xml-read-element (point-min) (point-max))
      ('incomplete nil)
      ('nil 'declined)
      (read read))))

(defconst jabber-test-xml--stanzas
  '("<message from='a@b/c' to=\"d@e\" type='chat' id='x1'><body>Hi &amp; bye &lt;3 &#x1F600;&#33;</body><active xmlns='http://jabber.org/protocol/chatstates'/></message>"
    "<presence from='room@muc/nick'><x xmlns='http://jabber.org/protocol/muc#user'><item affiliation='none' role='participant'/></x><c xmlns='http://jabber.org/protocol/caps' hash='sha-1' node='n' ver='v='/></presence>"
    "<iq type='result' id='q1'><fin xmlns='urn:xmpp:mam:2' complete='true'><set xmlns='http://jabber.org/protocol/rsm'><first>1</first><last>2</last></set></fin></iq>"
    "<message><result xmlns='urn:xmpp:mam:2' queryid='q' id='9'><forwarded xmlns='urn:xmpp:forward:0'><delay xmlns='urn:xmpp:delay' stamp='2026-11-05T10:00:00Z'/><message type='groupchat' from='r@m/n'><body>line one\r\nline two\rthree\n  indented</body></message></forwarded></result></message>"
    "<message>\n  <body>  spaced  </body>\n  <html xmlns='http://jabber.org/protocol/xhtml-im'><body xmlns='http://www.w3.org/1999/xhtml'><p>a <b>b</b> c</p></body></html>\n</message>"
    "<message a='1'b='2' c = \"3\"><body>café 日本 مرحبا</body></message>"
    "<a><b><b><b/></b></b><![CDATA[raw <x> &amp; ]]>after &gt;<c/><![CDATA[]]></a>"
    "<a x='&quot;q&quot; &apos;s&apos; &#60;&#x3e;'/>"
    "<a><!-- comment --><b/></a>"
    "<a>&nbsp;</a>"
    "<message><body>hello</bodyx ></message><presence/>"
    "<a>&#0;</a>"
    "  \n <presence/>")
  "Stanzas the native reader must read as xml.el does, or decline.")

(defun jabber-test-xml--random-stanza (state depth)
  "A random stanza from the random STATE, nested at most DEPTH deep."
  (let* ((names ["a" "b" "body" "x:y" "message" "item_1" "q.r-s"])
         (texts ["" " " "\n" "t" "a &amp; b" "&lt;&gt;" "&#65;&#x42;" "éè" "\r\n" "x\ry"])
         (pick (lambda (v) (aref v (cl-random (length v) state))))
         (name (funcall pick names))
         (attrs (mapconcat (lambda (_) (format " %s%s='%s'"
                                               (funcall pick ["k" "id" "xmlns" "x:k"]) (cl-random 99 state)
                                               (funcall pick ["v" "&amp;" "\"q\"" "" "é" "a b"])))
                           (number-sequence 1 (cl-random 3 state)) "")))
    (if (or (zerop depth) (zerop (cl-random 3 state)))
        (format "<%s%s/>" name attrs)
      (format "<%s%s>%s</%s>" name attrs
              (mapconcat (lambda (_)
                           (pcase (cl-random 4 state)
                             (0 (funcall pick texts))
                             (1 (format "<![CDATA[%s]]>%s" (funcall pick ["" "<b>" "]" "a&b"]) (funcall pick texts)))
                             (_ (jabber-test-xml--random-stanza state (1- depth)))))
                         (number-sequence 1 (cl-random 5 state)) "")
              name))))

(ert-deftest jabber-test-xml-native-reads-as-xml-el ()
  "The native reader reads each stanza, and each of its prefixes, as
xml.el does: the same element and end, nothing yet, or it declines."
  (skip-unless (fboundp 'romacs--xml-read-element))
  (let ((state (cl-make-random-state 7))
        (stanzas (copy-sequence jabber-test-xml--stanzas)))
    (dotimes (_ 200)
      (push (jabber-test-xml--random-stanza state 4) stanzas))
    (dolist (stanza stanzas)
      (dolist (end (number-sequence 1 (length stanza)))
        (let* ((text (substring stanza 0 end))
               (native (jabber-test-xml--natively text)))
          (unless (eq native 'declined)
            (should (equal (list text (jabber-test-xml--by-xml-el text))
                           (list text native)))))))))

(ert-deftest jabber-test-xml-native-declines-what-xml-el-reads-otherwise ()
  "Comments, unknown entities and mismatched end tags are left to xml.el."
  (skip-unless (fboundp 'romacs--xml-read-element))
  (dolist (stanza '("<a><!-- c --><b/></a>" "<a>&nbsp;</a>"
                    "<message><body>hello</bodyx ></message><presence/>"))
    (should (eq 'declined (jabber-test-xml--natively stanza)))
    ;; And the stanza reader gives xml.el's answer.
    (with-temp-buffer
      (insert stanza)
      (should (equal (let ((r (jabber-test-xml--by-xml-el stanza))) (and r (list (car r))))
                     (jabber-xml-parse-next-stanza))))))

(provide 'jabber-test-xml)
;;; jabber-test-xml.el ends here
