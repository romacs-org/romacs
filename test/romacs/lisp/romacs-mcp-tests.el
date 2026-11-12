;;; romacs-mcp-tests.el --- Native MCP regressions -*- lexical-binding: t; -*-
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-mcp-companion)

(ert-deftest romacs-mcp-bundled-entrypoints ()
  (should (fboundp 'romacs-mcp-start))
  (should (fboundp 'romacs-mcp-stop))
  (should (fboundp 'romacs-mcp-register-tool)))

(ert-deftest romacs-mcp-owner-fence-before-effect ()
  (let ((romacs-mcp-test-effect nil))
    (should-error
     (romacs-mcp--eval
      (romacs-mcp--object "instance" "wrong" "code"
                           "(setq romacs-mcp-test-effect t)")))
    (should-not romacs-mcp-test-effect)))

(ert-deftest romacs-mcp-eval-all-forms-and-unicode ()
  (should (equal "\"Ελληνικά\\n42\""
                 (romacs-mcp--eval
                  (romacs-mcp--object
                   "instance" (gethash "instance" (romacs-mcp-identity))
                   "code" "(setq romacs-mcp-test-value 42) (format \"Ελληνικά\\n%s\" romacs-mcp-test-value)")))))

(ert-deftest romacs-mcp-registry-validation ()
  (should-error (romacs-mcp-register-tool "" "Bad" nil #'ignore))
  (should-error (romacs-mcp-register-tool "bad" "Bad" (romacs-mcp--object "type" "object") nil)))

(ert-deftest romacs-mcp-invalid-schema-arguments ()
  (let ((schema (romacs-mcp--schema '(("code" . "string")) '("code"))))
    (should-error (romacs-mcp--arguments schema (romacs-mcp--object "code" 7)))
    (should-error (romacs-mcp--arguments schema (romacs-mcp--object)))
    (should-error (romacs-mcp--arguments schema (romacs-mcp--object "code" "ok" "extra" 1)))))

(ert-deftest romacs-mcp-companion-preserves-study ()
  (romacs-mcp-enable-companion-tools)
  (let* ((study (current-buffer))
         (point (point))
         (identity (gethash "instance" (romacs-mcp-identity)))
         (claim (romacs-mcp--companion-claim (romacs-mcp--object "instance" identity)))
         (args (romacs-mcp--object "instance" identity
                                    "handle" (gethash "handle" claim)
                                    "operationId" "ert-edit" "tick" (gethash "tick" claim)
                                    "start" 1 "end" 1 "text" "study\nΕλλάδα")))
    (unwind-protect
        (progn
          (should (equal "succeeded" (gethash "status" (romacs-mcp--companion-edit args))))
          (should (equal "succeeded" (gethash "status" (romacs-mcp--companion-edit args))))
          (should (eq study (current-buffer)))
          (should (= point (point))))
      (kill-buffer (gethash "buffer" claim)))))

(ert-deftest romacs-mcp-nested-json-unicode ()
  (let ((romacs-mcp-tools nil))
    (romacs-mcp-register-tool
     "unicode" "Fixture" (romacs-mcp--schema nil nil)
     (lambda (_) (romacs-mcp--object "text" "Ελλάδα\n")))
    (let* ((result (romacs-mcp--call (romacs-mcp--object "name" "unicode") t))
           (wire (json-serialize result :false-object :false)))
      (should (equal "Ελλάδα\n"
                     (gethash "text" (json-parse-string
                                      (gethash "text" (aref (gethash "content" result) 0))))))
      (should (stringp wire)))))

(ert-deftest romacs-mcp-stable-boot-and-owned-socket ()
  (let* ((root (make-temp-file "romacs-mcp-ert-" t))
         (socket (expand-file-name "mcp" root))
         (boot (gethash "instance" (romacs-mcp-identity))))
    (unwind-protect
        (progn
          (romacs-mcp-start socket)
          (should (file-exists-p socket))
          (should-error (romacs-mcp-start socket))
          (romacs-mcp-stop)
          (should-not (file-exists-p socket))
          (romacs-mcp-start socket)
          (should (equal boot (gethash "instance" (romacs-mcp-identity))))
          (delete-file socket)
          (with-temp-file socket (insert "successor"))
          (romacs-mcp-stop)
          (should (file-exists-p socket)))
      (romacs-mcp-stop)
      (delete-directory root t))))

(ert-deftest romacs-mcp-refuse-existing-node-and-symlink ()
  (let* ((root (make-temp-file "romacs-mcp-node-" t))
         (socket (expand-file-name "mcp" root)))
    (unwind-protect
        (progn
          (with-temp-file socket (insert "not a socket"))
          (should-error (romacs-mcp-start socket))
          (delete-file socket)
          (make-symbolic-link (expand-file-name "absent" root) socket)
          (should-error (romacs-mcp-start socket)))
      (delete-directory root t))))

(ert-deftest romacs-mcp-reclaim-stale-socket-only ()
  "A socket left by a dead listener is reclaimed; a live one is refused.
Like GNU `server-start', which deletes a leftover socket unless
`server-running-p' can connect to it."
  (let* ((root (make-temp-file "romacs-mcp-stale-" t))
         (socket (expand-file-name "mcp" root))
         live)
    (unwind-protect
        (progn
          ;; A crashed editor leaves its socket node behind with no listener.
          (delete-process (make-network-process
                           :name "romacs-mcp-ert-dead" :family 'local
                           :service socket :server t :noquery t))
          (should (file-exists-p socket))
          (romacs-mcp-start socket)
          (should (process-live-p romacs-mcp--listener))
          (should (equal (romacs-mcp--socket-id socket) romacs-mcp--socket-identity))
          (romacs-mcp-stop)
          (should-not (file-exists-p socket))
          ;; Another live listener keeps its socket.
          (setq live (make-network-process
                      :name "romacs-mcp-ert-live" :family 'local
                      :service socket :server t :noquery t))
          (let ((identity (romacs-mcp--socket-id socket)))
            (should-error (romacs-mcp-start socket))
            (should-not romacs-mcp--listener)
            (should (process-live-p live))
            (should (equal identity (romacs-mcp--socket-id socket)))))
      (when live (delete-process live))
      (romacs-mcp-stop)
      (delete-directory root t))))

(ert-deftest romacs-mcp-reclaim-refuses-unconfirmed-stale-socket ()
  "Only a refused connection proves a socket stale.
A live listener whose socket cannot be connected to (permission denied)
keeps its node."
  (let* ((root (make-temp-file "romacs-mcp-stale-" t))
         (socket (expand-file-name "mcp" root))
         (live (make-network-process
                :name "romacs-mcp-ert-live" :family 'local
                :service socket :server t :noquery t))
         (identity (romacs-mcp--socket-id socket)))
    (unwind-protect
        (progn
          (set-file-modes socket 0)
          (should-error (romacs-mcp-start socket))
          (should-not romacs-mcp--listener)
          (should (process-live-p live))
          (should (equal identity (romacs-mcp--socket-id socket))))
      (delete-process live)
      (romacs-mcp-stop)
      (delete-directory root t))))

(ert-deftest romacs-mcp-reclaim-reads-only-the-failure-reason ()
  "A socket path containing \"refused\" does not make a denial look stale."
  (let* ((root (make-temp-file "romacs-mcp-stale-" t))
         (socket (expand-file-name "refused" root))
         (live (make-network-process
                :name "romacs-mcp-ert-live" :family 'local
                :service socket :server t :noquery t))
         (identity (romacs-mcp--socket-id socket)))
    (unwind-protect
        (progn
          (set-file-modes socket 0)
          (should-not (romacs-mcp--stale-socket-id socket))
          (should-error (romacs-mcp-start socket))
          (should (process-live-p live))
          (should (equal identity (romacs-mcp--socket-id socket))))
      (delete-process live)
      (romacs-mcp-stop)
      (delete-directory root t))))

(ert-deftest romacs-mcp-reclaim-deletes-only-the-node-it-checked ()
  "A node replaced right after its type check is not deleted.
The replacement, a regular file, also refuses connections, so only
checking type, owner and deletion identity on one stat keeps it."
  (let* ((root (make-temp-file "romacs-mcp-stale-" t))
         (socket (expand-file-name "mcp" root))
         (file-attributes-fn (symbol-function 'file-attributes))
         replaced)
    (unwind-protect
        (progn
          (delete-process (make-network-process
                           :name "romacs-mcp-ert-dead" :family 'local
                           :service socket :server t :noquery t))
          (cl-letf (((symbol-function 'file-attributes)
                     (lambda (name &optional id-format)
                       (prog1 (funcall file-attributes-fn name id-format)
                         (when (and (equal name socket) (not replaced))
                           (setq replaced t)
                           (delete-file socket)
                           (with-temp-file socket (insert "successor")))))))
            (should-error (romacs-mcp-start socket)))
          (should replaced)
          (should-not romacs-mcp--listener)
          (should (equal "successor"
                         (with-temp-buffer
                           (insert-file-contents socket)
                           (buffer-string)))))
      (romacs-mcp-stop)
      (delete-directory root t))))

(ert-deftest romacs-mcp-reclaim-never-deletes-a-successor ()
  "A socket that another editor replaced after the probe is not deleted."
  (let* ((root (make-temp-file "romacs-mcp-stale-" t))
         (socket (expand-file-name "mcp" root))
         successor identity)
    (cl-flet ((replace-after-probe (stale)
                ;; Another editor reclaims the same stale node first.
                (when stale
                  (delete-file socket)
                  (setq successor (make-network-process
                                   :name "romacs-mcp-ert-successor"
                                   :family 'local :service socket
                                   :server t :noquery t)
                        identity (romacs-mcp--socket-id socket)))
                stale))
      (unwind-protect
          (progn
            (delete-process (make-network-process
                             :name "romacs-mcp-ert-dead" :family 'local
                             :service socket :server t :noquery t))
            (advice-add 'romacs-mcp--stale-socket-id
                        :filter-return #'replace-after-probe)
            (should-error (romacs-mcp-start socket))
            (should-not romacs-mcp--listener)
            (should (process-live-p successor))
            (should (equal identity (romacs-mcp--socket-id socket))))
        (advice-remove 'romacs-mcp--stale-socket-id #'replace-after-probe)
        (when successor (delete-process successor))
        (romacs-mcp-stop)
        (delete-directory root t)))))

(defun romacs-mcp-test--pump (predicate)
  "Accept process output until PREDICATE returns non-nil."
  (with-timeout (10 (ert-fail "Condition not reached within 10 seconds"))
    (while (not (funcall predicate))
      (accept-process-output nil 0.05))))

(ert-deftest romacs-mcp-connections-leave-no-buffers ()
  "Accepted, rejected and stopped connections leave no buffer behind.
Like GNU `server-start', the listener has a filter, so
`server_accept_connection' gives accepted connections none."
  (let* ((root (make-temp-file "romacs-mcp-buffers-" t))
         (socket (expand-file-name "mcp" root))
         (line (concat (json-serialize
                        (romacs-mcp--object
                         "jsonrpc" "2.0" "id" 1 "method" "initialize" "params"
                         (romacs-mcp--object
                          "protocolVersion" "2025-06-18"
                          "capabilities" (romacs-mcp--object)
                          "clientInfo" (romacs-mcp--object))))
                       "\n"))
         (replies 0)
         (before (buffer-list))
         clients)
    (cl-flet ((connect ()
                (let ((client (make-network-process
                               :name "romacs-mcp-ert-client" :family 'local
                               :service socket :coding 'utf-8 :noquery t
                               :filter (lambda (_ chunk)
                                         (cl-incf replies (cl-count ?\n chunk))))))
                  (push client clients)
                  (process-send-string client line)
                  client))
              (new-buffers ()
                ;; GNU creates internal buffers such as
                ;; " *code-conversion-work*" lazily; they are not per-connection.
                (cl-remove-if (lambda (buffer)
                                (or (memq buffer before)
                                    (string-prefix-p " " (buffer-name buffer))))
                              (buffer-list))))
      (unwind-protect
          (progn
            (romacs-mcp-start socket)
            (dotimes (i 3)
              (let ((client (connect)))
                (romacs-mcp-test--pump (lambda () (= replies (1+ i))))
                (should (= 1 (length romacs-mcp--peers)))
                (delete-process client)
                (romacs-mcp-test--pump (lambda () (null romacs-mcp--peers)))
                (should-not (new-buffers))))
            ;; Fill the quota, then one more connection is rejected.  Wait
            ;; for each accept: a connect beyond the listen backlog blocks.
            (setq replies 0 clients nil)
            (dotimes (i (1+ romacs-mcp--peer-limit))
              (connect)
              (romacs-mcp-test--pump
               (lambda () (or (= (length romacs-mcp--peers) (1+ i))
                              (cl-some (lambda (c) (not (process-live-p c))) clients)))))
            (romacs-mcp-test--pump
             (lambda () (and (= replies romacs-mcp--peer-limit)
                             (= 1 (cl-count-if-not #'process-live-p clients)))))
            (should (= romacs-mcp--peer-limit (length romacs-mcp--peers)))
            (should-not (cl-some #'process-buffer romacs-mcp--peers))
            (should-not (new-buffers))
            (mapc #'delete-process clients)
            (romacs-mcp-test--pump (lambda () (null romacs-mcp--peers)))
            (should-not (new-buffers))
            ;; Stopping with connections still open leaves none either.
            (setq replies 0 clients nil)
            (dotimes (_ 2) (connect))
            (romacs-mcp-test--pump (lambda () (= replies 2)))
            (romacs-mcp-stop)
            (should-not romacs-mcp--peers)
            (should-not (new-buffers)))
        (mapc #'delete-process clients)
        (romacs-mcp-stop)
        (delete-directory root t)))))

(ert-deftest romacs-mcp-close-keeps-adopted-user-buffers ()
  "Closing a connection never kills a buffer it merely adopted.
A connection accepted by an earlier version of this library, whose
listener had no filter, adopted any existing buffer named like the
listener, as `server_accept_connection' uses `get-buffer-create'.
Like `delete-process', retiring it must leave that buffer alone."
  (let* ((root (make-temp-file "romacs-mcp-adopt-" t))
         (socket (expand-file-name "mcp" root))
         (buffers (list (generate-new-buffer "romacs-mcp <1>")
                        (generate-new-buffer "romacs-mcp <2>")))
         clients)
    (dolist (buffer buffers)
      (with-current-buffer buffer (insert "User draft: preserve me.\n")))
    (unwind-protect
        (progn
          (romacs-mcp-start socket)
          (dotimes (i 2)
            (push (make-network-process
                   :name "romacs-mcp-ert-client" :family 'local
                   :service socket :noquery t :filter #'ignore)
                  clients)
            (romacs-mcp-test--pump (lambda () (= (length romacs-mcp--peers) (1+ i))))
            (set-process-buffer (cl-find-if-not #'process-buffer romacs-mcp--peers)
                                (nth i buffers)))
          ;; One peer disconnects; the other is retired by stop.
          (delete-process (cadr clients))
          (romacs-mcp-test--pump (lambda () (= 1 (length romacs-mcp--peers))))
          (romacs-mcp-stop)
          (should-not romacs-mcp--peers)
          (dolist (buffer buffers)
            (should (buffer-live-p buffer))
            (should (buffer-modified-p buffer))
            (should (equal "User draft: preserve me.\n"
                           (with-current-buffer buffer (buffer-string))))))
      (mapc #'delete-process clients)
      (romacs-mcp-stop)
      (mapc #'kill-buffer buffers)
      (delete-directory root t))))

(ert-deftest romacs-mcp-filter-never-evaluates ()
  (let ((calls nil))
    (cl-letf (((symbol-function 'romacs-mcp--enqueue)
               (lambda (_peer message &optional failure) (push (or failure message) calls)))
              ((symbol-function 'romacs-mcp--dispatch)
               (lambda (&rest _) (ert-fail "Filter executed tool"))))
      (romacs-mcp--frame nil (encode-coding-string
                              "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/list\"}" 'utf-8))
      (romacs-mcp--frame nil "{bad}")
      (should (= 2 (length calls)))
      (should (= -32700 (car (car calls)))))))

(ert-deftest romacs-mcp-tool-notification-does-not-mutate ()
  (let ((calls nil))
    (cl-letf (((symbol-function 'romacs-mcp--enqueue)
               (lambda (&rest _) (setq calls t))))
      (romacs-mcp--frame nil "{\"jsonrpc\":\"2.0\",\"method\":\"tools/call\",\"params\":{}}")
      (should-not calls))))

(ert-deftest romacs-mcp-drain-fairness-and-nested-guard ()
  (let* ((peer 'fixture-peer)
         (romacs-mcp--active nil)
         (romacs-mcp--timer nil)
         (romacs-mcp--queue
          (mapcar (lambda (id)
                    (list :peer peer :generation romacs-mcp--generation
                          :cancelled nil :message (romacs-mcp--object
                                                   "id" id "method" "fixture")))
                  '(1 2 3 4 5 6)))
         (calls 0) (scheduled 0))
    (cl-letf (((symbol-function 'romacs-mcp--live-p) (lambda (&rest _) t))
              ((symbol-function 'input-pending-p) (lambda (&optional _) nil))
              ((symbol-function 'romacs-mcp--send) #'ignore)
              ((symbol-function 'romacs-mcp--schedule) (lambda () (cl-incf scheduled)))
              ((symbol-function 'romacs-mcp--dispatch)
               (lambda (&rest _)
                 (cl-incf calls)
                 (romacs-mcp--drain)
                 (romacs-mcp--object))))
      (romacs-mcp--drain)
      (should (= 1 calls))
      (should (= 5 (length romacs-mcp--queue)))
      (should (= 1 scheduled))
      (should-not romacs-mcp--active))))

(ert-deftest romacs-mcp-peer-queue-quota ()
  (let ((romacs-mcp--queue (make-list 16 (list :peer 'peer)))
        (closed nil))
    (cl-letf (((symbol-function 'romacs-mcp--close) (lambda (peer) (setq closed peer))))
      (romacs-mcp--enqueue 'peer (romacs-mcp--object))
      (should (eq closed 'peer))
      (should (= 16 (length romacs-mcp--queue))))))

(ert-deftest romacs-mcp-malformed-code-not-silently-truncated ()
  (let ((romacs-mcp-test-effect nil))
    (should-error
     (romacs-mcp--eval
      (romacs-mcp--object
       "instance" (gethash "instance" (romacs-mcp-identity))
       "code" ") (setq romacs-mcp-test-effect t)")))
    (should-not romacs-mcp-test-effect)))

(ert-deftest romacs-mcp-legacy-metadata-preserves-era ()
  (cl-letf (((symbol-function 'process-get) (lambda (_peer _key) 'ready)))
    (dolist (meta (list (romacs-mcp--object)
                        (romacs-mcp--object "progressToken" "ert-progress")
                        (romacs-mcp--object "example.com/context" "fixture")))
      (should (= 0 (hash-table-count
                    (romacs-mcp--dispatch
                     'peer (romacs-mcp--object "method" "ping" "params"
                                               (romacs-mcp--object "_meta" meta)))))))))

(ert-deftest romacs-mcp-dual-era-explicit-modern-validation ()
  (cl-letf (((symbol-function 'process-get) (lambda (_peer _key) 'ready)))
    (dolist (key '("io.modelcontextprotocol/protocolVersion"
                   "io.modelcontextprotocol/clientCapabilities"
                   "io.modelcontextprotocol/clientInfo"))
      (should-error
       (romacs-mcp--dispatch
        'peer (romacs-mcp--object "method" "ping" "params"
                                  (romacs-mcp--object "_meta"
                                                       (romacs-mcp--object key :null))))
       :type 'romacs-mcp-protocol-error))))

(ert-deftest romacs-mcp-unsupported-version-exact-data ()
  (let* ((failure
          (should-error
           (romacs-mcp--modern-p
            (romacs-mcp--object
             "_meta" (romacs-mcp--object
                      "io.modelcontextprotocol/protocolVersion" "1900-01-01"
                      "io.modelcontextprotocol/clientCapabilities" (romacs-mcp--object))))
           :type 'romacs-mcp-protocol-error))
         (data (nth 3 failure)))
    (should (= -32022 (nth 1 failure)))
    (should (equal "1900-01-01" (gethash "requested" data)))
    (should (equal ["2026-07-28" "2025-11-25" "2025-06-18"] (gethash "supported" data)))
    (should (= 2 (hash-table-count data)))))

(ert-deftest romacs-mcp-modern-ping-complete ()
  (cl-letf (((symbol-function 'process-get) (lambda (_peer _key) 'ready)))
    (let ((result
           (romacs-mcp--dispatch
            'peer (romacs-mcp--object
                   "method" "ping" "params"
                   (romacs-mcp--object
                    "_meta" (romacs-mcp--object
                             "io.modelcontextprotocol/protocolVersion" romacs-mcp--modern
                             "io.modelcontextprotocol/clientCapabilities" (romacs-mcp--object)))))))
      (should (equal "complete" (gethash "resultType" result))))))

;; Keep the established public ERT entry point inclusive of optional reads.
(require 'romacs-mcp-responsive-tests)
(require 'romacs-mcp-negotiation-tests)
(require 'romacs-mcp-reply-tests)

(provide 'romacs-mcp-tests)
;;; romacs-mcp-tests.el ends here
