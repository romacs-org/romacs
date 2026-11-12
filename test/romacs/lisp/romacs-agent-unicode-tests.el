;;; romacs-agent-unicode-tests.el --- Non-ASCII at agent JSON boundaries -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)

;; romacs-agent brings romacs-agent-tools, which installs its own tools and
;; call function: loading this file leaves the endpoint's as they were.
(defconst romacs-agent-unicode-tests--registrations
  (list romacs-mcp-call-function (copy-alist romacs-mcp-tools)))
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(ert-deftest romacs-agent-unicode-tests-leave-the-endpoint-as-it-was ()
  (should (eq romacs-mcp-call-function (car romacs-agent-unicode-tests--registrations)))
  (should (equal romacs-mcp-tools (cadr romacs-agent-unicode-tests--registrations))))

(ert-deftest romacs-mcp-unicode-keeps-valid-text-and-replaces-one-for-one ()
  (let ((text "it’s café 中文 😀 é"))
    (should (eq (romacs-mcp--unicode text) text)))
  (dolist (char (list (unibyte-char-to-multibyte 255) #xD800 #xDFFF #x110000))
    (let ((text (string ?a char ?b)))
      (should (equal (romacs-mcp--unicode text) (string ?a #xFFFD ?b)))
      (should (= (length (romacs-mcp--unicode text)) (length text)))))
  ;; Unibyte text is bytes, one character each as its buffer counts them:
  ;; a byte past ASCII, UTF-8 or not, is U+FFFD, and the length is kept.
  (should (equal (romacs-mcp--unicode (unibyte-string ?a #xC3 #xA9 ?b)) (string ?a #xFFFD #xFFFD ?b)))
  (should (equal (romacs-mcp--unicode (unibyte-string ?a 255 ?b)) (string ?a #xFFFD ?b)))
  (should (equal (romacs-mcp--unicode (unibyte-string ?a ?b)) "ab"))
  ;; Literal text that looks like an escape stays itself.
  (should (equal (romacs-mcp--unicode "a\\xffb") "a\\xffb")))

(ert-deftest romacs-mcp-unicode-value-cleans-every-json-representation ()
  (let* ((raw (string ?x (unibyte-char-to-multibyte 200)))
         (clean (string ?x #xFFFD))
         (table (make-hash-table :test #'equal)))
    (puthash raw raw table)
    (let ((value (romacs-mcp--unicode-value table)))
      (should (equal (gethash clean value) clean)))
    (should (equal (romacs-mcp--unicode-value (vector raw 1)) (vector clean 1)))
    (should (equal (romacs-mcp--unicode-value (list :a raw :b 2)) (list :a clean :b 2)))
    (should (equal (romacs-mcp--unicode-value (list (cons (quote k) raw))) (list (cons (quote k) clean))))
    (dolist (value (list table (vector raw) (list :a raw) (list (cons (quote k) raw))))
      (should (json-serialize (romacs-mcp--unicode-value value) :false-object :false :null-object :null)))
    ;; A value that serializes is returned as it is.
    (let ((fine (list :a "café")))
      (should (eq (romacs-mcp--unicode-value fine) fine)))
    ;; A circular one is left to be refused, at once.
    (let ((circular (list :a raw)))
      (setcdr (cdr circular) circular)
      (should (eq (romacs-mcp--unicode-value circular) circular)))))

(ert-deftest romacs-mcp-result-with-a-raw-byte-is-sent ()
  (let* ((result (romacs-mcp--result (string ?a (unibyte-char-to-multibyte 255) ?b) nil t))
         (text (gethash "text" (aref (gethash "content" result) 0))))
    (should (equal text (string ?a #xFFFD ?b)))
    (should (json-serialize result :false-object :false :null-object :null))))

(ert-deftest romacs-agent-json-is-text-and-nests-once ()
  (let* ((object (romacs-mcp--object "type" "user" "text" "it’s café 😀"))
         (text (romacs-agent--json object)))
    (should (multibyte-string-p text))
    (should (equal (gethash "text" (json-parse-string text)) "it’s café 😀"))
    ;; The supervisor's envelope: the line is text inside it, serialized once.
    (let ((envelope (json-parse-string (json-serialize (romacs-mcp--object "write" text)))))
      (should (equal (gethash "text" (json-parse-string (gethash "write" envelope))) "it’s café 😀")))))

(ert-deftest romacs-agent-tools-call-keys-and-log-survive-non-ascii ()
  ;; Loaded for its call log alone: what loading it installs (its tools,
  ;; the call function, its hooks) is put back.
  (let ((romacs-mcp-call-function romacs-mcp-call-function)
        (romacs-mcp-tools (copy-alist romacs-mcp-tools))
        (romacs-agent-event-functions romacs-agent-event-functions)
        (romacs-agent-restored-hook romacs-agent-restored-hook))
    (require 'romacs-agent-tools))
  (let* ((state (make-temp-file "romacs-agent-unicode-" t))
         (process-environment (cons (concat "XDG_STATE_HOME=" state) process-environment))
         (romacs-agent-tools--calls (make-hash-table :test #'equal))
         (peer (make-pipe-process :name "romacs-agent-unicode-peer" :noquery t))
         (session "44444444-5555-4666-8777-888888888888"))
    (unwind-protect
        (progn
          (process-put peer 'romacs-agent-relay "relay-é")
          (let ((key (romacs-agent-tools--key peer "café")))
            (should (equal key "relay-é:\"café\""))
            (make-directory (romacs-agent-tools--dir session) t)
            (romacs-agent-tools--note session key (romacs-mcp--object "state" "done" "result" "résultat 中"))
            ;; As after a restart: the log read anew.
            (clrhash romacs-agent-tools--calls)
            (let ((record (gethash key (romacs-agent-tools--table session))))
              (should record)
              (should (equal (gethash "result" record) "résultat 中")))))
      (delete-process peer)
      (delete-directory state t))))

(provide 'romacs-agent-unicode-tests)
;;; romacs-agent-unicode-tests.el ends here
