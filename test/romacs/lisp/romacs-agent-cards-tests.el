;;; romacs-agent-cards-tests.el --- Delegations: their wakes, their cards, the header -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-history))

(defconst romacs-agent-cards-tests--parent "11111111-2222-4333-8444-555555555555")
(defconst romacs-agent-cards-tests--child "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee")

(defmacro romacs-agent-cards-tests--with-chat (var &rest body)
  "BODY with a session VAR with one known child."
  (declare (indent 1))
  `(let* ((romacs-agent-glyphs nil)
          (,var (romacs-agent--make :id romacs-agent-cards-tests--parent :harness 'claude :state 'running
                                    :title "parent" :tools (make-hash-table :test #'equal)))
          (child (romacs-agent--make :id romacs-agent-cards-tests--child :harness 'claude :state 'idle :title "child"
                                     :options (list :parent romacs-agent-cards-tests--parent))))
     (puthash (romacs-agent-id child) child romacs-agent--sessions)
     (unwind-protect (with-temp-buffer ,@body)
       (remhash (romacs-agent-id child) romacs-agent--sessions))))

(ert-deftest romacs-agent-wake-text-reads-back ()
  (romacs-agent-cards-tests--with-chat session
    (ignore session)
    (setf (romacs-agent-title child) "a (nested) title")
    (should (equal (romacs-agent-tools--wake-parse (romacs-agent-tools--wake-text child '(1 "r\ns" t)))
                   (list :id romacs-agent-cards-tests--child :title "a (nested) title" :state "failed" :reply-to nil
                         :result "r\ns")))
    ;; A turn that answered a message names it.
    (should (equal (plist-get (romacs-agent-tools--wake-parse (romacs-agent-tools--wake-text child '(1 "r" nil "m-12ab")))
                              :reply-to)
                   "m-12ab"))
    (should-not (romacs-agent-tools--wake-parse "hello"))))

(ert-deftest romacs-agent-header-says-only-what-is-worth-a-glance ()
  (romacs-agent-cards-tests--with-chat session
    (setf (romacs-agent-model session) "claude-opus-5-5" (romacs-agent-permission-mode session) "bypassPermissions"
          (romacs-agent-options session) '(:effort "high")
          (romacs-agent-context-used session) 440000 (romacs-agent-context-window session) 1000000
          (romacs-agent-provider session) "claude")
    (let ((romacs-agent-glyphs nil) (romacs-agent-default-provider "claude"))
      (cl-flet ((header () (substring-no-properties
                            (romacs-agent-ui--header-text (romacs-agent-ui--header-inputs session)))))
        (should (equal (header) (concat (romacs-agent-ui--harness-icon 'claude) "  Working  opus-5-5 high  44%")))
        ;; A mode other than bypassing approvals is worth a glance.
        (setf (romacs-agent-permission-mode session) "plan")
        (should (string-search "  plan  " (header)))
        ;; Another account of the harness is named only on hover: the harness is what shows.
        (setf (romacs-agent-provider session) "claude2")
        (should-not (string-search "claude2" (header)))
        (let ((text (romacs-agent-ui--header-text (romacs-agent-ui--header-inputs session))))
          (should (equal (get-text-property (string-search "opus-5-5" text) 'help-echo text) "account claude2")))))))

(ert-deftest romacs-agent-history-shows-a-delegation-card ()
  (let ((file (make-temp-file "romacs-cards" nil ".jsonl")) (buffer nil) (romacs-agent-glyphs nil))
    (unwind-protect
        (progn
          (with-temp-file file
            (insert "{\"type\":\"assistant\",\"message\":{\"id\":\"m1\",\"content\":[{\"type\":\"tool_use\",\"id\":\"t1\","
                    "\"name\":\"mcp__romacs__romacs_delegate_task\",\"input\":{\"title\":\"Review it\"}}]}}\n"
                    "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"t1\","
                    "\"content\":\"{\\\"taskId\\\":\\\"" romacs-agent-cards-tests--child "\\\",\\\"status\\\":\\\"running\\\"}\"}]}}\n"))
          (setq buffer (romacs-agent-history-open (list :path file :title "h" :harness 'claude :id "h1" :cwd "/")))
          (with-current-buffer buffer
            ;; Drawn as a chat draws it: the delegation's line, its call's mark.
            (should (eq major-mode 'romacs-agent-history-mode))
            (should (string-match-p "⇢ Review it\n" (buffer-string)))))
      (when (buffer-live-p buffer) (kill-buffer buffer))
      (delete-file file))))

;; Runs: the steps between words, one row each.

(provide 'romacs-agent-cards-tests)
;;; romacs-agent-cards-tests.el ends here
