;;; romacs-agent-environment-tests.el --- A harness's environment -*- lexical-binding: t; -*-
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(defconst romacs-agent-environment-tests--launch
  '("ROMACS_RUNTIME_ROOT" "ROMACS_LISP_DIR" "ROMACS_USAGE_LOG" "ROMACS_SOURCE_DIRECTORY" "ROMACS_NOT_YET_INVENTED")
  "The editor's variables, the ones a hand list once named, one it missed,
and one no list has heard of.")

(ert-deftest romacs-agent-harness-environment-drops-launch-variables-and-shares-nothing ()
  (dolist (placement '(absent first last))
    (let* ((launch (mapcar (lambda (name) (concat name "=editor")) romacs-agent-environment-tests--launch))
           (rest (list "KEEP_ME=editor" "OTHER=1"))
           (process-environment (pcase placement
                                  ('absent (copy-sequence rest))
                                  ('first (append launch (copy-sequence rest)))
                                  ('last (append (copy-sequence rest) launch))))
           (environment (romacs-agent--harness-environment)))
      (dolist (name romacs-agent-environment-tests--launch)
        (should-not (let ((process-environment environment)) (getenv name)))
        ;; Unset by name in a unit too, when the editor holds it.
        (should (eq (and (memq placement '(first last)) t)
                    (and (member name (romacs-agent--dropped-variables)) t))))
      ;; A look-alike that is not the editor's stays.
      (let ((process-environment (cons "XROMACS_KEEP=1" process-environment)))
        (should (equal (let ((process-environment (romacs-agent--harness-environment))) (getenv "XROMACS_KEEP")) "1")))
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

(ert-deftest romacs-agent-eval-keeps-its-budget-when-the-endpoint-reloads ()
  ;; Loading romacs-mcp.el again (a live load) keeps the agent layer's
  ;; romacs_eval, budgetMs and all; with none registered it gives its own.
  (let ((romacs-mcp-tools (copy-alist romacs-mcp-tools)))
    (cl-letf (((symbol-function 'run-at-time) #'ignore))
      (romacs-mcp-register-tool "romacs_eval" "Budgeted."
                                (romacs-mcp--schema '(("code" . "string") ("budgetMs" . "integer")) '("code"))
                                #'ignore)
      (load "romacs-mcp" nil t)
      (should (equal (plist-get (cdr (assoc "romacs_eval" romacs-mcp-tools)) :description) "Budgeted."))
      ;; Accepted: an unknown argument would signal.
      (romacs-mcp--arguments (plist-get (cdr (assoc "romacs_eval" romacs-mcp-tools)) :schema)
                             (romacs-mcp--object "code" "1" "budgetMs" 10))
      (setq romacs-mcp-tools (assoc-delete-all "romacs_eval" romacs-mcp-tools))
      (load "romacs-mcp" nil t)
      (should (eq (plist-get (cdr (assoc "romacs_eval" romacs-mcp-tools)) :handler) #'romacs-mcp--eval)))))

(provide 'romacs-agent-environment-tests)
;;; romacs-agent-environment-tests.el ends here
