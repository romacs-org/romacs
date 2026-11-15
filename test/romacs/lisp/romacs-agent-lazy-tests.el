;;; romacs-agent-lazy-tests.el --- A call's output drawn when opened -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)
(require 'romacs-test-socket)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-ui))

(defconst romacs-agent-lazy-tests--id "7e570000-3333-4444-8555-666677778888")

(defun romacs-agent-lazy-tests--line (seq key value)
  (concat (json-serialize (romacs-mcp--object key value "seq" seq "t" (+ 1794600000.25 seq))) "\n"))

(defun romacs-agent-lazy-tests--call (seq id name input)
  "An assistant record at SEQ starting call ID of NAME on INPUT."
  (romacs-agent-lazy-tests--line
   seq "out" (romacs-mcp--object
              "message" (romacs-mcp--object
                         "content" (vector (romacs-mcp--object "id" id "input" input "name" name "type" "tool_use"))
                         "id" (format "msg_%d" seq))
              "parent_tool_use_id" :null "type" "assistant" "uuid" "u")))

(defun romacs-agent-lazy-tests--result (seq id output &optional failed)
  "A user record at SEQ with call ID's OUTPUT, FAILED or not."
  (romacs-agent-lazy-tests--line
   seq "out" (romacs-mcp--object
              "message" (romacs-mcp--object
                         "content" (vector (romacs-mcp--object "content" output "is_error" (if failed t :false)
                                                               "tool_use_id" id "type" "tool_result"))
                         "role" "user")
              "parent_tool_use_id" :null "session_id" "s" "tool_use_result" (romacs-mcp--object "stdout" output)
              "type" "user" "uuid" "u")))

(defun romacs-agent-lazy-tests--with-chat (body)
  "Call BODY with a chat drawn from a journal of three calls: Bash ok, Bash
failed, Grep (whose row counts what it found)."
  (let* ((root (make-temp-file "romacs-lazy-" t))
         (run (romacs-test-socket-directory "lazy-"))
         (process-environment (append (list (concat "XDG_STATE_HOME=" root "/state") (concat "XDG_RUNTIME_DIR=" run))
                                      process-environment))
         (romacs-agent--sessions (make-hash-table :test #'equal))
         (dir (expand-file-name romacs-agent-lazy-tests--id (romacs-agent--state-root))))
    (unwind-protect
        (progn
          (make-directory dir t)
          (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                    (romacs-mcp--object "id" romacs-agent-lazy-tests--id "harness" "claude"
                                                        "provider" "claude" "cwd" root "title" "lazy" "titleAuto" :false
                                                        "argv" [] "options" (romacs-mcp--object) "seenSeq" 0
                                                        "hostSocket" (expand-file-name "none/host.sock" root)))
          (let ((coding-system-for-write 'utf-8-unix))
            (write-region
             (concat (romacs-agent-lazy-tests--line
                      1 "in" (romacs-mcp--object "message" (romacs-mcp--object "content" "go" "role" "user")
                                                 "romacsRequest" 1 "type" "user"))
                     (romacs-agent-lazy-tests--call 2 "t1" "Bash" (romacs-mcp--object "command" "ls"))
                     (romacs-agent-lazy-tests--result 3 "t1" "alpha é\nbeta")
                     (romacs-agent-lazy-tests--call 4 "t2" "Bash" (romacs-mcp--object "command" "false"))
                     (romacs-agent-lazy-tests--result 5 "t2" "exit 1" t)
                     (romacs-agent-lazy-tests--call 6 "t3" "Grep" (romacs-mcp--object "pattern" "x"))
                     (romacs-agent-lazy-tests--result 7 "t3" "a.el:1\nb.el:2")
                     (romacs-agent-lazy-tests--line 8 "out" (romacs-mcp--object "result" "done" "type" "result")))
             nil (expand-file-name "journal.jsonl" dir) nil 'silent))
          (cl-letf (((symbol-function 'romacs-agent--keep-models) #'ignore))
            (romacs-agent-restore)
            (let* ((session (romacs-agent-get romacs-agent-lazy-tests--id))
                   (buffer (romacs-agent-ui--make-chat session)))
              (unwind-protect (with-current-buffer buffer (funcall body session))
                (kill-buffer buffer)))))
      (delete-directory root t)
      (delete-directory run t))))

(ert-deftest romacs-agent-lazy-outputs-parse-and-draw-when-opened ()
  (romacs-agent-lazy-tests--with-chat
   (lambda (session)
     (let* ((parsed nil)
            (parse (symbol-function 'json-parse-string)))
       (cl-letf (((symbol-function 'json-parse-string)
                  (lambda (string &rest args)
                    (when (string-search "tool_result" string) (push string parsed))
                    (apply parse string args))))
         (romacs-agent-ui--fill (current-buffer)))
       ;; Only Grep's result was parsed: its row counts what it found.
       (should (= (length parsed) 1))
       (should (string-search "\"t3\"" (car parsed)))
       (let ((text (buffer-string)))
         (should (string-search "⌕ x · 2 lines" text))
         ;; The Bash outputs are not in the chat until opened.
         (should-not (string-search "alpha" text))
         (should-not (string-search "exit 1" text)))
       ;; TAB on a row opens its output, drawn from its record now.
       (goto-char (point-min))
       (search-forward "$ ls")
       (romacs-agent-chat-toggle)
       (let ((text (buffer-string)))
         (should (string-search "alpha é\nbeta" text))
         (should-not (string-search "exit 1" text)))
       (should-not (memq 'romacs-agent-output
                         (ensure-list (get-text-property (1+ (string-search "alpha" (buffer-string))) 'invisible))))
       ;; Again, it folds as any output does.
       (goto-char (point-min))
       (search-forward "$ ls")
       (romacs-agent-chat-toggle)
       (should (memq 'romacs-agent-output
                     (ensure-list (get-text-property (1+ (string-search "alpha" (buffer-string))) 'invisible))))
       ;; Expand all draws the rest.
       (romacs-agent-chat-expand-all)
       (should (string-search "exit 1" (buffer-string)))))))

(ert-deftest romacs-agent-lazy-pointer-finds-its-record ()
  ;; A pointer whose bytes are off finds its record by its seq.
  (romacs-agent-lazy-tests--with-chat
   (lambda (session)
     (should (equal (romacs-agent-ui--ref-output (list :session session :id "t1" :seq 3 :from 0 :to 10))
                    "alpha é\nbeta")))))

(ert-deftest romacs-agent-lazy-results-told-from-text ()
  ;; A result's own text blocks (an MCP tool's) are its; a block of the
  ;; message's own beside the results is not a result: that line parses.
  (let ((summary (lambda (line) (with-temp-buffer (insert line) (romacs-agent--claude-result-summary 1 (1- (point-max)))))))
    (let ((mcp (romacs-agent-lazy-tests--line
                9 "out" (romacs-mcp--object
                         "message" (romacs-mcp--object
                                    "content" (vector (romacs-mcp--object
                                                       "content" (vector (romacs-mcp--object "text" "42" "type" "text"))
                                                       "tool_use_id" "t9" "type" "tool_result"))
                                    "role" "user")
                         "parent_tool_use_id" "toolu_parent" "type" "user" "uuid" "u"))))
      (should (equal (mapcar (lambda (r) (list (plist-get r :id) (plist-get r :failed) (plist-get r :parent) (plist-get r :seq)))
                             (funcall summary mcp))
                     '(("t9" nil "toolu_parent" 9)))))
    (let ((mixed (romacs-agent-lazy-tests--line
                  10 "out" (romacs-mcp--object
                            "message" (romacs-mcp--object
                                       "content" (vector (romacs-mcp--object "content" "ok" "tool_use_id" "t10" "type" "tool_result")
                                                         (romacs-mcp--object "text" "[Request interrupted]" "type" "text"))
                                       "role" "user")
                            "parent_tool_use_id" :null "type" "user" "uuid" "u"))))
      (should-not (funcall summary mixed)))
    ;; Two results, the second failed.
    (let ((two (romacs-agent-lazy-tests--line
                11 "out" (romacs-mcp--object
                          "message" (romacs-mcp--object
                                     "content" (vector (romacs-mcp--object "content" "a" "tool_use_id" "t1" "type" "tool_result")
                                                       (romacs-mcp--object "content" "b" "is_error" t "tool_use_id" "t2" "type" "tool_result"))
                                     "role" "user")
                          "parent_tool_use_id" :null "type" "user" "uuid" "u"))))
      (should (equal (mapcar (lambda (r) (list (plist-get r :id) (plist-get r :failed))) (funcall summary two))
                     '(("t1" nil) ("t2" t)))))))

(ert-deftest romacs-agent-lazy-scan-keeps-to-the-message ()
  ;; A result-shaped object in the record's own metadata after the message
  ;; (`tool_use_result') is no result of the message's.
  (let* ((line (romacs-agent-lazy-tests--line
                12 "out" (romacs-mcp--object
                          "message" (romacs-mcp--object
                                     "content" (vector (romacs-mcp--object "content" "ok" "tool_use_id" "t1" "type" "tool_result"))
                                     "role" "user")
                          "parent_tool_use_id" "toolu_p"
                          "tool_use_result" (romacs-mcp--object
                                             "inner" (romacs-mcp--object "content" "x" "is_error" t
                                                                         "tool_use_id" "t9" "type" "tool_result"))
                          "type" "user" "uuid" "u")))
         (results (with-temp-buffer (insert line) (romacs-agent--claude-result-summary 1 (1- (point-max))))))
    (should (equal (mapcar (lambda (r) (list (plist-get r :id) (plist-get r :failed) (plist-get r :parent))) results)
                   '(("t1" nil "toolu_p"))))))

(ert-deftest romacs-agent-lazy-copy-trim-and-expand ()
  (romacs-agent-lazy-tests--with-chat
   (lambda (_session)
     (romacs-agent-ui--fill (current-buffer))
     ;; Copying the turn copies outputs not yet opened, as before.
     (goto-char (point-min))
     (search-forward "$ ls")
     (romacs-agent-chat-copy)
     (should (string-search "alpha é" (current-kill 0)))
     ;; Rows trimmed away take their payloads with them.
     (romacs-agent-ui--open-payloads-in (point-min) (point-max))
     (let ((romacs-agent-ui--tool-lines (make-hash-table :test #'equal)))
       (puthash "x" '(:output "kept") (or romacs-agent-ui--payloads (setq romacs-agent-ui--payloads (make-hash-table :test #'equal))))
       ;; Opened with no row, a payload is forgotten.
       (should-not (romacs-agent-ui--open-payload "x"))
       (should-not (gethash "x" romacs-agent-ui--payloads))))))

;;; romacs-agent-lazy-tests.el ends here
