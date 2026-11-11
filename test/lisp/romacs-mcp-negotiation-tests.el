;;; romacs-mcp-negotiation-tests.el --- Legacy MCP negotiation -*- lexical-binding: t; -*-
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(defvar romacs-mcp-test-negotiation-effect)

(defmacro romacs-mcp-test-with-peer (&rest body)
  "Run BODY with a fresh process-property fixture named peer."
  (declare (indent 0) (debug t))
  `(let ((peer (make-process :name "mcp-negotiation-fixture"
                             :command '("cat") :connection-type 'pipe
                             :noquery t)))
     (unwind-protect (progn ,@body)
       (when (process-live-p peer) (delete-process peer)))))

(defun romacs-mcp-test-initialize (peer version)
  "Initialize PEER with the offered protocol VERSION."
  (romacs-mcp--dispatch
   peer (romacs-mcp--object
         "method" "initialize" "params"
         (romacs-mcp--object "protocolVersion" version
                              "capabilities" (romacs-mcp--object)
                              "clientInfo" (romacs-mcp--object
                                            "name" "codex-mcp-client"
                                            "version" "0.155.1")))))

(ert-deftest romacs-mcp-negotiation-supported-legacy-offers ()
  (dolist (version '("2025-06-18" "2025-11-25"))
    (romacs-mcp-test-with-peer
      (let ((result (romacs-mcp-test-initialize peer version)))
        (should (equal version (gethash "protocolVersion" result)))
        (should (hash-table-p (gethash "tools" (gethash "capabilities" result))))
        (should-not (gethash "resultType" result))
        (should (eq 'initializing (process-get peer 'legacy)))))))

(ert-deftest romacs-mcp-negotiation-unsupported-offer-counterproposal ()
  ;; The handshake negotiates legacy semantics, never stateless-modern semantics.
  (dolist (version '("2024-11-05" "1900-01-01" "2026-07-28"))
    (romacs-mcp-test-with-peer
      (should (equal "2025-11-25"
                     (gethash "protocolVersion"
                              (romacs-mcp-test-initialize peer version))))
      (should (eq 'initializing (process-get peer 'legacy))))))

(ert-deftest romacs-mcp-negotiation-malformed-does-not-initialize ()
  (dolist (params
           (append
            (mapcar (lambda (version)
                      (romacs-mcp--object "protocolVersion" version
                                           "capabilities" (romacs-mcp--object)
                                           "clientInfo" (romacs-mcp--object)))
                    '(:null 20250618 ""))
            (list (romacs-mcp--object)
                  (romacs-mcp--object "capabilities" (romacs-mcp--object)
                                       "clientInfo" (romacs-mcp--object))
                  (romacs-mcp--object "protocolVersion" "2025-06-18"
                                       "capabilities" :null
                                       "clientInfo" (romacs-mcp--object))
                  (romacs-mcp--object "protocolVersion" "2025-06-18"
                                       "capabilities" (romacs-mcp--object)
                                       "clientInfo" :null))))
    (romacs-mcp-test-with-peer
      (let ((failure
             (should-error
              (romacs-mcp--dispatch
               peer (romacs-mcp--object "method" "initialize" "params" params))
              :type 'romacs-mcp-protocol-error)))
        (should (= -32602 (nth 1 failure)))
        (should-not (process-get peer 'legacy))
        ;; A malformed offer must not poison a later valid handshake.
        (should (equal "2025-06-18"
                       (gethash "protocolVersion"
                                (romacs-mcp-test-initialize peer "2025-06-18"))))))))

(ert-deftest romacs-mcp-negotiation-readiness-and-repeated-initialize ()
  (dolist (version '("2025-06-18" "2025-11-25"))
    (romacs-mcp-test-with-peer
      (should-error
       (romacs-mcp--dispatch peer (romacs-mcp--object "method" "notifications/initialized"))
       :type 'romacs-mcp-protocol-error)
      (should-not (process-get peer 'legacy))
      (romacs-mcp-test-initialize peer version)
      (let ((failure
             (should-error
              (romacs-mcp--dispatch peer (romacs-mcp--object "method" "tools/list"))
              :type 'romacs-mcp-protocol-error)))
        (should (= -32600 (nth 1 failure))))
      (should-error (romacs-mcp-test-initialize peer version)
                    :type 'romacs-mcp-protocol-error)
      (should (eq 'initializing (process-get peer 'legacy)))
      (romacs-mcp--dispatch peer (romacs-mcp--object "method" "notifications/initialized"))
      (should (eq 'ready (process-get peer 'legacy)))
      (should-error (romacs-mcp-test-initialize peer "2025-11-25")
                    :type 'romacs-mcp-protocol-error)
      (should-error
       (romacs-mcp--dispatch peer (romacs-mcp--object "method" "notifications/initialized"))
       :type 'romacs-mcp-protocol-error)
      (should (eq 'ready (process-get peer 'legacy)))
      (should (= 0 (hash-table-count
                    (romacs-mcp--dispatch peer (romacs-mcp--object "method" "ping"))))))))

(ert-deftest romacs-mcp-negotiation-tools-list-changed ()
  ;; Advertised, and sent once to each initialized peer however many
  ;; tools a command registers.
  (romacs-mcp-test-with-peer
    (let* ((romacs-mcp--peers (list peer))
           (romacs-mcp-tools (copy-alist romacs-mcp-tools))
           (romacs-mcp--tools-telling nil)
           (scheduled 0) (sent nil)
           (result (romacs-mcp-test-initialize peer "2025-06-18")))
      (should (eq t (gethash "listChanged" (gethash "tools" (gethash "capabilities" result)))))
      (cl-letf (((symbol-function 'run-at-time) (lambda (&rest _) (cl-incf scheduled) nil))
                ((symbol-function 'romacs-mcp--send)
                 (lambda (request response) (push (cons (plist-get request :peer) (gethash "method" response)) sent))))
        ;; Not yet initialized: not told.
        (romacs-mcp-register-tool "romacs_test_one" "One." (romacs-mcp--schema nil nil) #'ignore)
        (romacs-mcp--tell-tools-changed)
        (should-not sent)
        (romacs-mcp--dispatch peer (romacs-mcp--object "method" "notifications/initialized"))
        (setq scheduled 0)
        (romacs-mcp-register-tool "romacs_test_one" "Replaced." (romacs-mcp--schema nil nil) #'ignore)
        (romacs-mcp-register-tool "romacs_test_two" "Two." (romacs-mcp--schema nil nil) #'ignore)
        (should (= scheduled 1))
        (romacs-mcp--tell-tools-changed)
        (should (equal sent (list (cons peer "notifications/tools/list_changed"))))))))

(ert-deftest romacs-mcp-negotiation-subscriptions-listen ()
  ;; A stateless client listens for the tool list's changes: acknowledged
  ;; at once, held open, told with its subscription's id until cancelled.
  (romacs-mcp-test-with-peer
    (let* ((romacs-mcp--peers (list peer))
           (romacs-mcp-tools (copy-alist romacs-mcp-tools))
           (romacs-mcp--subscriptions nil) (romacs-mcp--tools-telling nil) (sent nil)
           (meta (romacs-mcp--object "io.modelcontextprotocol/protocolVersion" romacs-mcp--modern
                                     "io.modelcontextprotocol/clientInfo" (romacs-mcp--object "name" "c" "version" "1")
                                     "io.modelcontextprotocol/clientCapabilities" (romacs-mcp--object)))
           (message (romacs-mcp--object "jsonrpc" "2.0" "id" 7 "method" "subscriptions/listen"
                                        "params" (romacs-mcp--object "_meta" meta "notifications"
                                                                     (romacs-mcp--object "toolsListChanged" t))))
           (listen (list :peer peer :generation romacs-mcp--generation :message message :cancelled nil :held nil)))
      (should (eq t (gethash "listChanged"
                             (gethash "tools" (gethash "capabilities"
                                                       (romacs-mcp--dispatch
                                                        peer (romacs-mcp--object "method" "server/discover"
                                                                                 "params" (romacs-mcp--object "_meta" meta))))))))
      (cl-letf (((symbol-function 'run-at-time) #'ignore)
                ((symbol-function 'romacs-mcp--send)
                 (lambda (_request response)
                   (let ((params (gethash "params" response)))
                     (push (list (gethash "method" response)
                                 (gethash "io.modelcontextprotocol/subscriptionId" (gethash "_meta" params)))
                           sent)))))
        (let ((romacs-mcp--active listen)) (romacs-mcp--dispatch peer message))
        (should (plist-get listen :held))
        (should (equal sent '(("notifications/subscriptions/acknowledged" 7))))
        (romacs-mcp-register-tool "romacs_test_late" "Late." (romacs-mcp--schema nil nil) #'ignore)
        (romacs-mcp--tell-tools-changed)
        (should (equal (car sent) '("notifications/tools/list_changed" 7)))
        ;; Cancelled by the client: told nothing more, and forgotten.
        (romacs-mcp--cancel peer 7)
        (romacs-mcp--tell-tools-changed)
        (should (= (length sent) 2))
        (should-not romacs-mcp--subscriptions)))))

(ert-deftest romacs-mcp-negotiation-legacy-tool-envelopes-and-fence ()
  (dolist (version '("2025-06-18" "2025-11-25"))
    (romacs-mcp-test-with-peer
      (romacs-mcp-test-initialize peer version)
      (romacs-mcp--dispatch peer (romacs-mcp--object "method" "notifications/initialized"))
      (let* ((listed (romacs-mcp--dispatch peer (romacs-mcp--object "method" "tools/list")))
             (instance (gethash "instance" (romacs-mcp-identity)))
             (romacs-mcp-test-negotiation-effect nil)
             (params (romacs-mcp--object
                      "name" "romacs_eval" "arguments"
                      (romacs-mcp--object "instance" "wrong" "code"
                                           "(setq romacs-mcp-test-negotiation-effect t)")))
             (wrong (romacs-mcp--dispatch
                     peer (romacs-mcp--object "method" "tools/call" "params" params))))
        (should (vectorp (gethash "tools" listed)))
        (should-not (gethash "resultType" listed))
        (should (eq t (gethash "isError" wrong)))
        (should-not (gethash "resultType" wrong))
        (should-not romacs-mcp-test-negotiation-effect)
        (puthash "arguments" (romacs-mcp--object "instance" instance "code" "(+ 20 22)") params)
        (let ((result (romacs-mcp--dispatch
                       peer (romacs-mcp--object "method" "tools/call" "params" params))))
          (should (eq :false (gethash "isError" result)))
          (should (equal "42" (gethash "text" (aref (gethash "content" result) 0))))
          (should-not (gethash "resultType" result)))))))

(ert-deftest romacs-mcp-negotiation-modern-discovery-all-versions ()
  (romacs-mcp-test-with-peer
    (let* ((params (romacs-mcp--object
                    "_meta" (romacs-mcp--object
                             "io.modelcontextprotocol/protocolVersion" "2026-07-28"
                             "io.modelcontextprotocol/clientCapabilities" (romacs-mcp--object))))
           (result (romacs-mcp--dispatch
                    peer (romacs-mcp--object "method" "server/discover" "params" params))))
      (should (equal ["2026-07-28" "2025-11-25" "2025-06-18"]
                     (gethash "supportedVersions" result)))
      (should (equal "complete" (gethash "resultType" result)))
      (should-not (process-get peer 'legacy)))))

(provide 'romacs-mcp-negotiation-tests)
;;; romacs-mcp-negotiation-tests.el ends here
