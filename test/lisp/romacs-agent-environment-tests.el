;;; romacs-agent-environment-tests.el --- A harness's environment -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(ert-deftest romacs-agent-harness-environment-drops-launch-variables-and-shares-nothing ()
  (dolist (placement '(absent first last))
    (let* ((launch (mapcar (lambda (name) (concat name "=editor")) romacs-agent--editor-variables))
           (rest (list "KEEP_ME=editor" "OTHER=1"))
           (process-environment (pcase placement
                                  ('absent (copy-sequence rest))
                                  ('first (append launch (copy-sequence rest)))
                                  ('last (append (copy-sequence rest) launch))))
           (environment (romacs-agent--harness-environment)))
      (dolist (name romacs-agent--editor-variables)
        (should-not (let ((process-environment environment)) (getenv name))))
      (should (equal (let ((process-environment environment)) (getenv "KEEP_ME")) "editor"))
      ;; A setenv under it leaves the editor's variable as it was.
      (let ((process-environment environment)) (setenv "KEEP_ME" "eval"))
      (should (equal (getenv "KEEP_ME") "editor")))))

(ert-deftest romacs-agent-eval-runs-without-launch-variables ()
  (require 'romacs-agent-tools)
  (let ((process-environment (append (list "ROMACS_USAGE_LOG=/stale" "KEEP_ME=editor") process-environment)))
    (cl-letf (((symbol-function 'romacs-mcp--instance) #'ignore)
              ((symbol-function 'romacs--call-deadline) #'ignore))
      (let ((arguments (romacs-mcp--object "code" "(list (getenv \"ROMACS_USAGE_LOG\") (getenv \"KEEP_ME\") (setenv \"KEEP_ME\" \"eval\"))"
                                           "budgetMs" 0)))
        (should (equal (romacs-agent-tools--eval arguments) "(nil \"editor\" \"eval\")"))
        (should (equal (getenv "KEEP_ME") "editor"))
        (should (equal (getenv "ROMACS_USAGE_LOG") "/stale"))))))

(provide 'romacs-agent-environment-tests)
;;; romacs-agent-environment-tests.el ends here
