;;; romacs-agent-refusals-tests.el --- Routes after refused agent tool calls -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'cl-lib)
(require 'json)

(defconst romacs-agent-refusals-tests--sha256
  "d6ca10a3428d57b1b091f07b670b6e70a438cde9d79e86ff6a81d62db1a9b962"
  "SHA-256 of the issue log extract used by these tests.")

(defconst romacs-agent-refusals-tests--corpus
  (with-temp-buffer
    (set-buffer-multibyte nil)
    (insert-file-contents-literally
     (expand-file-name "romacs-agent-refusals-corpus.json"
                       (file-name-directory (or load-file-name buffer-file-name))))
    (let ((digest (secure-hash 'sha256 (current-buffer))))
      (unless (equal digest romacs-agent-refusals-tests--sha256)
        (error "Refusal corpus SHA-256 mismatch: expected %s, got %s"
               romacs-agent-refusals-tests--sha256 digest)))
    (json-parse-string (decode-coding-string (buffer-string) 'utf-8) :array-type 'list))
  "Recorded tool refusals, with the issue log's argument summaries.")

(require 'romacs-mcp)
(require 'romacs-agent)
(require 'romacs-agent-codex)

(defmacro romacs-agent-refusals-tests--with (&rest body)
  "Run BODY with a private agent registry, store, and caller session."
  (declare (indent 0))
  `(let* ((root (file-name-as-directory (make-temp-file "agent-refusals-" t)))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent-store--db nil)
          (romacs-agent-watch--watches (make-hash-table :test #'equal))
          (romacs-agent-providers '(("codex" :harness codex)))
          (romacs-mcp--active nil)
          (romacs-mcp--boot "refusals-test-instance")
          (romacs-mcp-full-access t)
          (process-environment (copy-sequence process-environment))
          (caller (romacs-agent-refusals-tests--session "caller" root)))
     (setenv "XDG_RUNTIME_DIR" root)
     (unwind-protect
         (cl-letf (((symbol-function 'romacs-agent--state-root) (lambda () root))
                   ((symbol-function 'romacs-agent-tools--caller)
                    (lambda () (romacs-agent-id caller))))
           ,@body)
       (when (cdr romacs-agent-store--db)
         (sqlite-close (cdr romacs-agent-store--db)))
       (delete-directory root t))))

(defun romacs-agent-refusals-tests--session (id cwd &optional parent)
  "Register a private session with ID, CWD and optional PARENT."
  (puthash id (romacs-agent--make :id id :harness 'codex :provider "codex"
                                  :title id :state 'idle :cwd cwd
                                  :dir (expand-file-name id cwd)
                                  :options (list :parent parent))
           romacs-agent--sessions))

(defun romacs-agent-refusals-tests--case (group tool error-fragment)
  "Return the first GROUP record for TOOL mentioning ERROR-FRAGMENT."
  (or (cl-find-if (lambda (record)
                    (and (equal group (gethash "group" record))
                         (equal tool (gethash "tool" record))
                         (string-search error-fragment (gethash "error" record))))
                  romacs-agent-refusals-tests--corpus)
      (ert-fail (list "Missing refusal fixture" group tool error-fragment))))

(defun romacs-agent-refusals-tests--call (record &rest replacements)
  "Call RECORD's tool with its arguments, applying key-value REPLACEMENTS."
  (let ((arguments (copy-hash-table (gethash "arguments" record))))
    (cl-loop for (key value) on replacements by #'cddr do (puthash key value arguments))
    (let* ((params (romacs-mcp--object "name" (gethash "tool" record) "arguments" arguments))
           (result (romacs-mcp--call params nil)))
      (if (not (eq (car-safe result) 'romacs-mcp--deferred)) result
        (let* ((request (list :message (romacs-mcp--object "params" params)))
               (romacs-mcp--deferred (list (list request 1 (nth 1 result) (nth 3 result) (nth 2 result))))
               (romacs-mcp--deferred-index (make-hash-table :test #'equal))
               (reply nil))
          (cl-letf (((symbol-function 'romacs-mcp--live-p) (lambda (&rest _) t))
                    ((symbol-function 'romacs-mcp--send) (lambda (_request answer) (setq reply answer))))
            (romacs-mcp--poll-deferred))
          (gethash "result" reply))))))

(defun romacs-agent-refusals-tests--text (result)
  "Read the single text item in an MCP RESULT."
  (gethash "text" (aref (gethash "content" result) 0)))

(defun romacs-agent-refusals-tests--refusal (record &rest replacements)
  "Call RECORD and return its refusal, with argument REPLACEMENTS applied."
  (let* ((refused nil)
         (romacs-mcp-failure-functions
          (list (lambda (_peer _tool _arguments _message reason) (setq refused reason)))))
    (condition-case failure
        (let ((result (apply #'romacs-agent-refusals-tests--call record replacements)))
          (should (eq (gethash "isError" result) t))
          (should refused)
          (romacs-agent-refusals-tests--text result))
      (romacs-mcp-protocol-error
       (should (equal (nth 1 failure) -32602))
       (should refused)
       (nth 2 failure)))))

(defun romacs-agent-refusals-tests--names-p (text alternative)
  "Whether TEXT names the exact word or phrase ALTERNATIVE."
  (let ((case-fold-search nil))
    (string-match-p (concat "\\(?:\\`\\|[^[:alnum:]_-]\\)"
                            (regexp-quote alternative)
                            "\\(?:\\'\\|[^[:alnum:]_-]\\)")
                    text)))

(defun romacs-agent-refusals-tests--next-steps (&rest checks)
  "Require one named alternative for each (REFUSAL ALTERNATIVE...) in CHECKS."
  (let ((missing
         (cl-loop for (text . alternatives) in checks
                  unless (cl-some (lambda (name) (romacs-agent-refusals-tests--names-p text name))
                                  alternatives)
                  collect (list :expected-one-of alternatives :refusal text))))
    (should-not missing)))

(ert-deftest romacs-agent-refusals-1-task-prefix ()
  (romacs-agent-refusals-tests--with
    (let* ((record (romacs-agent-refusals-tests--case 1 "romacs_task_status" "636c02a2"))
           (prefix (gethash "taskId" (gethash "arguments" record)))
           (first (concat prefix "-0000-4000-8000-000000000001"))
           (second (concat prefix "-0000-4000-8000-000000000002"))
           (task (romacs-agent-refusals-tests--session first root (romacs-agent-id caller))))
      (romacs-agent-store-put task)
      (should (equal (romacs-agent-store-prefixed-ids prefix) (list first)))
      (let* ((unique (romacs-agent-refusals-tests--call record))
             (text (romacs-agent-refusals-tests--text unique))
             (resolved (if (eq (gethash "isError" unique) :false)
                           (gethash "taskId" (json-parse-string text))
                         text)))
        (romacs-agent-store-put
         (romacs-agent-refusals-tests--session second root (romacs-agent-id caller)))
        (should (equal (romacs-agent-store-prefixed-ids prefix) (list first second)))
        (let* ((ambiguous (romacs-agent-refusals-tests--refusal record))
               (named (cl-remove-if-not (lambda (id) (romacs-agent-refusals-tests--names-p ambiguous id))
                                        (list first second))))
          (ert-info ((format "Ambiguous prefix refusal: %s" ambiguous))
            (should (equal (list resolved named) (list first (list first second))))))))))

(ert-deftest romacs-agent-refusals-2-ownership ()
  (romacs-agent-refusals-tests--with
    (let* ((send-root (romacs-agent-refusals-tests--case 2 "romacs_task_send" "no delegated task"))
           (send-other (romacs-agent-refusals-tests--case 2 "romacs_task_send" "task of session"))
           (switch (romacs-agent-refusals-tests--case 2 "romacs_task_switch" "may move it"))
           (adopt (romacs-agent-refusals-tests--case 2 "romacs_delegate_task" "may take it over"))
           (root-id (gethash "taskId" (gethash "arguments" send-root)))
           (owner "bbbbbbbb-0000-4000-8000-000000000000")
           (conversation "cccccccc-0000-4000-8000-000000000000"))
      (romacs-agent-refusals-tests--session root-id root)
      (romacs-agent-refusals-tests--session owner root)
      (setf (romacs-agent-options caller) (list :parent root-id)
            (romacs-agent-received caller)
            (list (romacs-agent-make-message "m-aaaaaaaa-0000-4000-8000-000000000001"
                                             "parent" (list :session root-id) nil "Please answer")))
      (dolist (record (list send-other switch))
        (romacs-agent-refusals-tests--session (gethash "taskId" (gethash "arguments" record)) root owner))
      (romacs-agent-refusals-tests--session conversation root owner)
      (let ((unowned
             (list (romacs-agent-refusals-tests--refusal send-other)
                   (romacs-agent-refusals-tests--refusal switch)
                   (romacs-agent-refusals-tests--refusal adopt "resume" conversation "cwd" root))))
        (apply #'romacs-agent-refusals-tests--next-steps
               (cons (list (romacs-agent-refusals-tests--refusal send-root) "romacs_task_reply")
                     (cl-loop for text in unowned
                              append (list (list text owner) (list text "ask" "contact" "through")))))))))

(ert-deftest romacs-agent-refusals-3-schema ()
  (romacs-agent-refusals-tests--with
    (apply #'romacs-agent-refusals-tests--next-steps
           (cl-loop for (tool error . alternatives) in
                    '(("romacs_task_send" "Missing argument: message" "message")
                      ("romacs_task_send" "Missing argument: taskId" "taskId")
                      ("romacs_task_status" "Missing argument: taskId" "taskId")
                      ("romacs_task_reply" "Missing argument: text" "text")
                      ("romacs_delegate_task" "Missing argument: task" "task")
                      ("romacs_task_watch" "Unknown argument: log" "output")
                      ("romacs_task_watch" "Unknown argument: logFile" "output")
                      ("romacs_delegate_task" "Unknown argument: provider" "target.provider" "under target" "inside target")
                      ("romacs_delegate_task" "Unknown argument: model" "target.model" "under target" "inside target"))
                    for record = (romacs-agent-refusals-tests--case 3 tool error)
                    collect (cons (romacs-agent-refusals-tests--refusal record) alternatives)))))

(ert-deftest romacs-agent-refusals-4-stopped-unit ()
  (romacs-agent-refusals-tests--with
    (let* ((record (romacs-agent-refusals-tests--case 4 "romacs_task_watch" "avt3-watch-mine.service"))
           (unit (gethash "unit" (gethash "arguments" record)))
           (invocation "76c58ced7ce741669e018765c54ff31a"))
      (make-directory (expand-file-name "unit-exit" root))
      (with-temp-file (expand-file-name unit (expand-file-name "unit-exit" root))
        (insert "exit-code exited 7 " invocation "\n"))
      (cl-letf (((symbol-function 'romacs-agent-watch--output)
                 (lambda (_program args then)
                   (should (equal (car (last args)) unit))
                   (funcall then (format "Id=%s\nActiveState=inactive\nInvocationID=%s\n" unit invocation) 0))))
        (romacs-agent-refusals-tests--next-steps
         (list (romacs-agent-refusals-tests--refusal record "output" "output.log")
               "read its end" "status=7" "journalctl"))))))

(ert-deftest romacs-agent-refusals-5-message-id ()
  (romacs-agent-refusals-tests--with
    (let* ((short (romacs-agent-refusals-tests--case 5 "romacs_task_reply" "m-4863e4c4 among"))
           (unknown (romacs-agent-refusals-tests--case 5 "romacs_task_reply" "m-ea32a04c-"))
           (prefix (gethash "messageId" (gethash "arguments" short)))
           (first (concat prefix "-0000-4000-8000-000000000001"))
           (second (concat prefix "-0000-4000-8000-000000000002"))
           (other (concat prefix "-0000-4000-8000-000000000003"))
           (sender (romacs-agent-refusals-tests--session "sender" root)))
      (setf (romacs-agent-received caller)
            (mapcar (lambda (id) (romacs-agent-make-message id "parent" '(:session "sender") nil "Question"))
                    (list first second))
            (romacs-agent-received sender)
            (list (romacs-agent-make-message other "child" '(:session "caller") nil "Another question")))
      (let ((checks
             (cl-loop for record in (list short unknown)
                      for text = (romacs-agent-refusals-tests--refusal record)
                      append (progn
                               (should-not (romacs-agent-refusals-tests--names-p text other))
                               (list (list text first) (list text second))))))
        (apply #'romacs-agent-refusals-tests--next-steps checks)))))

(ert-deftest romacs-agent-refusals-6-bad-task-id ()
  (romacs-agent-refusals-tests--with
    (romacs-agent-refusals-tests--session "20c8c0e5-0000-4000-8000-000000000000"
                                          root (romacs-agent-id caller))
    (romacs-agent-refusals-tests--next-steps
     (list (romacs-agent-refusals-tests--refusal
            (romacs-agent-refusals-tests--case 6 "romacs_task_send" "20c8c0e5-placeholder"))
           "romacs_task_list")
     (list (romacs-agent-refusals-tests--refusal
            (romacs-agent-refusals-tests--case 6 "romacs_task_status" "No task"))
           "romacs_task_list"))))

(ert-deftest romacs-agent-refusals-7-instance ()
  (romacs-agent-refusals-tests--with
    (romacs-agent-refusals-tests--next-steps
     (list (romacs-agent-refusals-tests--refusal
            (romacs-agent-refusals-tests--case 7 "romacs_task_switch" "Romacs instance mismatch"))
           "romacs_identity")
     (list (romacs-agent-refusals-tests--refusal
            (romacs-agent-refusals-tests--case 7 "romacs_orchestrator_capabilities" "Romacs instance mismatch"))
           "romacs_identity"))))

(ert-deftest romacs-agent-refusals-8-steer-mode ()
  (romacs-agent-refusals-tests--with
    (let* ((send (romacs-agent-refusals-tests--case 8 "romacs_task_send" "mode queue"))
           (reply (romacs-agent-refusals-tests--case 8 "romacs_task_reply" "mode queue")))
      (romacs-agent-refusals-tests--session (gethash "taskId" (gethash "arguments" send))
                                            root (romacs-agent-id caller))
      (romacs-agent-refusals-tests--next-steps
       (list (romacs-agent-refusals-tests--refusal send) "mode queue")
       (list (romacs-agent-refusals-tests--refusal reply) "mode queue")))))

;;; romacs-agent-refusals-tests.el ends here
