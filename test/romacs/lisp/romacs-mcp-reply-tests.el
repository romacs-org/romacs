;;; romacs-mcp-reply-tests.el --- Error replies and full access -*- lexical-binding: t; -*-
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-mcp-companion)

(defvar romacs-mcp-test-reply-effect nil)

(defun romacs-mcp-test--call-line (id name arguments)
  "Return a modern tools/call request line for tool NAME with ARGUMENTS and ID."
  (concat
   (json-serialize
    (romacs-mcp--object
     "jsonrpc" "2.0" "id" id "method" "tools/call" "params"
     (romacs-mcp--object
      "name" name "arguments" arguments
      "_meta" (romacs-mcp--object
               "io.modelcontextprotocol/protocolVersion" romacs-mcp--modern
               "io.modelcontextprotocol/clientCapabilities" (romacs-mcp--object)))))
   "\n"))

(defun romacs-mcp-test--reply (client output line)
  "Send LINE from CLIENT and return the parsed reply collected in OUTPUT.
Return nil if the server closes the connection without replying."
  (setcar output "")
  (process-send-string client line)
  (with-timeout (10 (ert-fail "No reply within 10 seconds"))
    (while (and (process-live-p client) (not (string-search "\n" (car output))))
      (accept-process-output client 0.05)))
  (when (string-search "\n" (car output))
    (json-parse-string (car (split-string (car output) "\n")))))

(ert-deftest romacs-mcp-unsendable-response-is-error-reply ()
  ;; ESC prints raw but JSON-escapes to 6 bytes: 30000 of them print to
  ;; about 30 KB but encode to about 180 KB, over the response limit: an
  ;; error reply for the request, not a silent disconnect, and the
  ;; connection stays usable.  A raw byte, as in undecodable process
  ;; output, is sent as U+FFFD.
  (let* ((root (make-temp-file "romacs-mcp-reply-" t))
         (socket (expand-file-name "mcp" root))
         (instance (gethash "instance" (romacs-mcp-identity)))
         (romacs-mcp-tools (copy-sequence romacs-mcp-tools))
         (output (list ""))
         client)
    (romacs-mcp-register-tool
     "raw" "Fixture" (romacs-mcp--schema nil nil)
     (lambda (_) (string ?a (unibyte-char-to-multibyte 200) ?b)))
    (unwind-protect
        (progn
          (romacs-mcp-start socket)
          (setq client (make-network-process
                        :name "romacs-mcp-reply-client" :family 'local :service socket
                        :coding 'utf-8 :noquery t
                        :filter (lambda (_ chunk) (setcar output (concat (car output) chunk)))))
          (let ((reply (romacs-mcp-test--reply
                        client output
                        (romacs-mcp-test--call-line
                         1 "romacs_eval"
                         (romacs-mcp--object "instance" instance "code" "(make-string 30000 27)")))))
            (should reply)
            (should (equal 1 (gethash "id" reply)))
            (should (= -32603 (gethash "code" (gethash "error" reply)))))
          ;; A raw byte in a tool's text is U+FFFD, one for one: the result
          ;; is sent, not refused.
          (let ((reply (romacs-mcp-test--reply client output
                                               (romacs-mcp-test--call-line 2 "raw" (romacs-mcp--object)))))
            (should reply)
            (should (equal 2 (gethash "id" reply)))
            (should (equal (string ?a #xFFFD ?b)
                           (gethash "text" (aref (gethash "content" (gethash "result" reply)) 0)))))
          (let ((reply (romacs-mcp-test--reply
                        client output
                        (romacs-mcp-test--call-line
                         3 "romacs_eval"
                         (romacs-mcp--object "instance" instance "code" "(* 6 7)")))))
            (should reply)
            (should (equal "42" (gethash "text" (aref (gethash "content" (gethash "result" reply))
                                                      0))))))
      (when (process-live-p client) (delete-process client))
      (romacs-mcp-stop)
      (delete-directory root t))))

(defun romacs-mcp-test--legacy-initialize (client output)
  "Complete a legacy handshake for CLIENT whose replies collect in OUTPUT."
  (should (romacs-mcp-test--reply
           client output
           (concat (json-serialize
                    (romacs-mcp--object
                     "jsonrpc" "2.0" "id" 1 "method" "initialize" "params"
                     (romacs-mcp--object
                      "protocolVersion" "2025-06-18"
                      "capabilities" (romacs-mcp--object)
                      "clientInfo" (romacs-mcp--object))))
                   "\n")))
  (process-send-string
   client "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n"))

(ert-deftest romacs-mcp-near-limit-id-reply-stays-bounded ()
  ;; A request just under the input limit whose ID alone is near that
  ;; limit must not produce an error reply over the output limit.  The ID
  ;; is not echoed; the connection stays usable.
  (let* ((root (make-temp-file "romacs-mcp-id-" t))
         (socket (expand-file-name "mcp" root))
         (output (list ""))
         client)
    (unwind-protect
        (progn
          (romacs-mcp-start socket)
          (setq client (make-network-process
                        :name "romacs-mcp-id-client" :family 'local :service socket
                        :coding 'utf-8 :noquery t
                        :filter (lambda (_ chunk) (setcar output (concat (car output) chunk)))))
          (romacs-mcp-test--legacy-initialize client output)
          (dolist (id (list (make-string 131000 ?x)
                            (json-parse-string (make-string 131000 ?9))))
            (let* ((line (concat (json-serialize
                                  (romacs-mcp--object
                                   "jsonrpc" "2.0" "id" id "method" "tools/list"))
                                 "\n"))
                   (reply (progn
                            (should (<= (string-bytes line) romacs-mcp--frame-limit))
                            (romacs-mcp-test--reply client output line))))
              (should reply)
              (should (<= (string-bytes (car output)) romacs-mcp--output-limit))
              (should (eq :null (gethash "id" reply)))
              (should (= -32600 (gethash "code" (gethash "error" reply))))))
          (let ((reply (romacs-mcp-test--reply
                        client output
                        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n")))
            (should (equal 2 (gethash "id" reply)))
            (should (gethash "tools" (gethash "result" reply)))))
      (when (process-live-p client) (delete-process client))
      (romacs-mcp-stop)
      (delete-directory root t))))

(ert-deftest romacs-mcp-wire-never-exceeds-the-output-limit ()
  ;; Admission bounds IDs, but the encoder must hold the limit on its own:
  ;; an oversized or unencodable response with an oversized ID still
  ;; yields one bounded error line.
  (let ((id (make-string romacs-mcp--output-limit ?x)))
    (dolist (result (list (make-string romacs-mcp--output-limit ?y)
                          (string ?a (unibyte-char-to-multibyte 200))))
      (let* ((wire (romacs-mcp--wire
                    (romacs-mcp--object "jsonrpc" "2.0" "id" id "result" result)))
             (reply (json-parse-string wire)))
        (should (<= (string-bytes wire) romacs-mcp--output-limit))
        (should (string-suffix-p "\n" wire))
        (should (eq :null (gethash "id" reply)))
        (should (= -32603 (gethash "code" (gethash "error" reply))))))
    (let ((wire (romacs-mcp--wire
                 (romacs-mcp--object "jsonrpc" "2.0" "id" 7
                                      "result" (make-string romacs-mcp--output-limit ?y)))))
      (should (equal 7 (gethash "id" (json-parse-string wire)))))))

(defun romacs-mcp-test--listed-names ()
  "Return the names of the tools a client can currently list."
  (mapcar (lambda (tool) (gethash "name" tool)) (romacs-mcp--tool-list)))

(ert-deftest romacs-mcp-full-access-default-and-off ()
  (should (eq t (default-value 'romacs-mcp-full-access)))
  (should (custom-variable-p 'romacs-mcp-full-access))
  (let ((romacs-mcp-full-access nil)
        (instance (gethash "instance" (romacs-mcp-identity))))
    (setq romacs-mcp-test-reply-effect nil)
    (should-not (member "romacs_eval" (romacs-mcp-test--listed-names)))
    (should (member "romacs_identity" (romacs-mcp-test--listed-names)))
    (let ((failure
           (should-error
            (romacs-mcp--call
             (romacs-mcp--object
              "name" "romacs_eval" "arguments"
              (romacs-mcp--object "instance" instance
                                   "code" "(setq romacs-mcp-test-reply-effect t)"))
             nil)
            :type 'romacs-mcp-protocol-error)))
      (should (= -32602 (nth 1 failure))))
    (should-not romacs-mcp-test-reply-effect))
  (let ((romacs-mcp-full-access t))
    (should (member "romacs_eval" (romacs-mcp-test--listed-names)))))

(ert-deftest romacs-mcp-full-access-off-keeps-companion-tools ()
  (let ((romacs-mcp-tools (copy-sequence romacs-mcp-tools))
        (romacs-mcp-full-access nil))
    (romacs-mcp-enable-companion-tools)
    (dolist (name '("romacs_companion_claim" "romacs_companion_read"
                    "romacs_companion_edit" "romacs_companion_receipt"
                    "romacs_companion_undo" "romacs_companion_retire"))
      (should (member name (romacs-mcp-test--listed-names))))
    (should-not (member "romacs_eval" (romacs-mcp-test--listed-names)))))

(provide 'romacs-mcp-reply-tests)
;;; romacs-mcp-reply-tests.el ends here
