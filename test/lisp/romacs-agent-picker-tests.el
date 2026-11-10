;;; romacs-agent-picker-tests.el --- Choosing what a session runs on -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-tools)
  (require 'romacs-agent-ui))

(defmacro romacs-agent-picker--with (bindings &rest body)
  "BODY in a private state directory, with `root' its temporary root and
`program' an executable there; BINDINGS, of `let*', see them."
  (declare (indent 1))
  `(let* ((root (make-temp-file "romacs-picker-" t))
          (program (expand-file-name "fake-claude" root))
          (process-environment (cons (concat "XDG_STATE_HOME=" root) process-environment))
          ,@bindings)
     (ignore program)
     (with-temp-file program (insert "#!/bin/sh\n"))
     (set-file-modes program #o755)
     (unwind-protect (progn ,@body)
       (delete-directory root t))))

(defun romacs-agent-picker--answer (&rest models)
  "An `initialize' answer of MODELS, as Claude Code gives it."
  (let ((answer (romacs-mcp--object "models" (vconcat models)
                                    "account" (romacs-mcp--object "email" "someone@example.org"))))
    answer))

(defun romacs-agent-picker--model (value resolved label &optional efforts)
  (let ((model (romacs-mcp--object "value" value "resolvedModel" resolved "displayName" label)))
    (when efforts (puthash "supportedEffortLevels" (vconcat efforts) model))
    model))

(ert-deftest romacs-agent-picker-claude-models ()
  ;; The default alias is no model; an alias goes by what it resolves to.
  (should (equal (romacs-agent--claude-models
                  (romacs-agent-picker--answer
                   (romacs-agent-picker--model "default" "claude-opus-5-5" "Default (recommended)" '("low"))
                   (romacs-agent-picker--model "opus" "claude-opus-5-5" "Opus 5.5" '("low" "max"))
                   (romacs-agent-picker--model "claude-haiku-4-5" "claude-haiku-4-5" "Haiku 4.5")))
                 '(("claude-opus-5-5" :label "Opus 5.5" :efforts ("low" "max"))
                   ("claude-haiku-4-5" :label "Haiku 4.5" :efforts nil))))
  (should-not (romacs-agent--claude-models (romacs-mcp--object))))

(ert-deftest romacs-agent-picker-models-merge ()
  (romacs-agent-picker--with
      ((session (romacs-agent--make :id "s" :harness 'claude :provider "claude"))
       (romacs-agent-providers `(("claude" :program ,program)))
       (romacs-agent-models '((claude ("a" :label "Seed A" :efforts ("low" "high") :default-effort "high")
                                      ("b" :label "Seed B" :efforts ("low"))))))
    (romacs-agent--keep-models session '(("a" :label "Reported A" :efforts ("low")) ("c" :label "Reported C")
                                         ("b" :label "Reported B" :efforts nil)))
    (let ((instance (romacs-agent--configured "claude")))
      ;; What the harness says goes first; the catalogue fills in the rest.
      (let ((rows (romacs-agent--models instance)))
        (should (equal (mapcar #'car rows) '("a" "c" "b")))
        (should (equal (plist-get (cdr (assoc "a" rows)) :label) "Reported A"))
        (should (equal (plist-get (cdr (assoc "a" rows)) :efforts) '("low")))
        ;; The catalogue's default, gone from what the harness takes, is none.
        (should-not (plist-get (cdr (assoc "a" rows)) :default-effort))
        ;; No efforts reported stays none, whatever the catalogue says.
        (should-not (plist-get (cdr (assoc "b" rows)) :efforts)))
      ;; The instance's own go before both.
      (let ((rows (romacs-agent--models (append (list :models '(("c" :label "Mine"))) instance))))
        (should (equal (mapcar #'car rows) '("c" "a" "b")))
        (should (equal (plist-get (cdr (assoc "c" rows)) :label) "Mine")))
      ;; An effort the harness took away is not asked for or carried over.
      (should-not (romacs-agent--effort-fits-p instance "b" "low"))
      ;; Only the account's model rows are kept.
      (should-not (string-search "account" (with-temp-buffer
                                             (insert-file-contents (romacs-agent--models-file "claude"))
                                             (buffer-string))))
      ;; Another program, or the same file changed: what was kept no longer holds.
      (set-file-times program (time-add (current-time) 100))
      (should (equal (mapcar #'car (romacs-agent--models instance)) '("a" "b"))))))

(ert-deftest romacs-agent-picker-refresh-asks-once ()
  (romacs-agent-picker--with
      ((session (romacs-agent--make :id "s" :harness 'claude :provider "claude"))
       (romacs-agent-providers `(("claude" :program ,program)))
       (written nil))
    (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_session object &optional _journal) (push object written))))
      (romacs-agent--refresh-models session)
      (should (equal (gethash "subtype" (gethash "request" (car written))) "initialize"))
      (let* ((id (gethash "request_id" (car written)))
             (response (lambda (request-id body)
                         (romacs-mcp--object "type" "control_response"
                                             "response" (romacs-mcp--object "subtype" "success" "request_id" request-id
                                                                            "response" body))))
             (answer (romacs-agent-picker--answer (romacs-agent-picker--model "x" "model-x" "X" '("low")))))
        ;; Another's answer, as on a replay, is nothing to keep.
        (romacs-agent--claude-events session (funcall response "other" answer))
        (should-not (file-exists-p (romacs-agent--models-file "claude")))
        (romacs-agent--claude-events session (funcall response id answer))
        (should (assoc "model-x" (romacs-agent--models (romacs-agent--configured "claude"))))
        (should-not (romacs-agent-models-request session))
        ;; Kept: the next session does not ask.
        (setq written nil)
        (romacs-agent--refresh-models session)
        (should-not written)))))

(ert-deftest romacs-agent-picker-refused-answer-keeps-nothing ()
  (romacs-agent-picker--with
      ((session (romacs-agent--make :id "s" :harness 'claude :provider "claude"
                                    :models-request "r"))
       (romacs-agent-providers `(("claude" :program ,program))))
    (romacs-agent--claude-events
     session (romacs-mcp--object "type" "control_response"
                                 "response" (romacs-mcp--object "subtype" "error" "request_id" "r" "error" "no")))
    (should-not (file-exists-p (romacs-agent--models-file "claude")))
    (should-not (romacs-agent-models-request session))))

(ert-deftest romacs-agent-picker-rows ()
  (romacs-agent-picker--with
      ((claude (expand-file-name "claude" root))
       (romacs-agent-providers
        `(("claude" :display "Claude" :config-dir ,claude :program ,program :model "a" :effort "high")
          ("claude2" :display "Claude" :config-dir ,(expand-file-name "gone" root) :program ,program)
          ("claude3" :config-dir ,claude :program ,program :enabled nil)
          ("codex" :display "Codex" :config-dir ,claude :program ,program)))
       (romacs-agent-default-provider "codex")
       (romacs-agent-models '((claude ("a" :label "Model A" :efforts ("low" "high")))
                              (codex ("b" :label "Model B")))))
    (make-directory claude)
    (let* ((rows (romacs-agent--picker-rows)) (texts (mapcar #'car rows)))
      ;; The default instance first, the disabled not at all, alike ones apart.
      (should (equal texts '("Codex · default" "Codex · Model B" "Claude · default" "Claude · Model A"
                             "Claude (claude2) · default" "Claude (claude2) · Model A")))
      (should (equal (plist-get (cdr (assoc "Codex · default" rows)) :note) "harness's own"))
      (should (string-suffix-p "default" (plist-get (cdr (car rows)) :group)))
      (should (equal (plist-get (cdr (assoc "Claude · default" rows)) :note) "a · high"))
      (should (equal (plist-get (cdr (assoc "Claude · Model A" rows)) :note) "effort low...high"))
      (should (equal (plist-get (cdr (assoc "Claude (claude2) · default" rows)) :note) "dir missing"))
      (should (equal (plist-get (cdr (assoc "Claude · Model A" rows)) :model) "a"))
      (should-not (plist-get (cdr (assoc "Claude · default" rows)) :model)))))

(defun romacs-agent-picker--reader (answers asked)
  "A `completing-read' giving ANSWERS in turn, each prompt, group and
affixation it saw pushed on the list in cell ASKED."
  (lambda (prompt table &rest _)
    (let* ((metadata (cdr (completion-metadata "" table nil)))
           (group (alist-get 'group-function metadata))
           (affix (alist-get 'affixation-function metadata))
           (answer (pop (car answers))))
      (push (list prompt
                  (and group (funcall group answer nil))
                  (and affix (nth 2 (car (funcall affix (list answer)))))
                  (all-completions "" table))
            (car asked))
      answer)))

(ert-deftest romacs-agent-picker-selection ()
  (romacs-agent-picker--with
      ((romacs-agent-providers `(("claude" :config-dir ,root :program ,program)
                                 ("codex" :config-dir ,root :program ,program)))
       (romacs-agent-models '((claude ("a" :label "A" :efforts ("low" "high") :default-effort "high")
                                      ("b" :label "B" :efforts ("low")))
                              (codex ("c" :label "C"))))
       (answers (list nil)) (asked (list nil)))
    (cl-letf (((symbol-function 'completing-read) (romacs-agent-picker--reader answers asked)))
      ;; RET on a row: its model, no effort asked: the harness's.
      (setcar answers (list "Claude · A"))
      (should (equal (romacs-agent--read-selection nil) '("claude" :model "a")))
      (pcase-let ((`(,prompt ,group ,note ,all) (car (car asked))))
        (should (string-match-p "Agent.*Claude · default" prompt))
        (should (string-prefix-p "Claude  " group))
        (should (string-search "effort low...high" note))
        (should (equal (car all) "Claude · default")))
      ;; An instance's defaults: nothing named.
      (setcar answers (list "Codex · default"))
      (should (equal (romacs-agent--read-selection nil) '("codex")))
      ;; C-u asks for the effort, the model's default offered.
      (setcar answers (list "Claude · A" "low"))
      (setcar asked nil)
      (should (equal (romacs-agent--read-selection '(4)) '("claude" :model "a" :effort "low")))
      (should (string-match-p "Effort.*high" (car (car (car asked)))))
      (should (equal (nth 3 (car (car asked))) '("low" "high")))
      ;; No default anywhere: asked without C-u.
      (setcar answers (list "Claude · B" "low"))
      (should (equal (romacs-agent--read-selection nil) '("claude" :model "b" :effort "low")))
      ;; A model with no efforts asks none, even with C-u.
      (setcar answers (list "Codex · C"))
      (should (equal (romacs-agent--read-selection '(4)) '("codex" :model "c"))))))

(ert-deftest romacs-agent-picker-one-instance-asks-nothing ()
  (romacs-agent-picker--with
      ((romacs-agent-providers `(("claude" :config-dir ,root :program ,program)
                                 ("codex" :config-dir ,root :program ,program :enabled nil)))
       (answers (list (list "Claude · default"))) (asked (list nil)))
    (cl-letf (((symbol-function 'completing-read) (romacs-agent-picker--reader answers asked)))
      (should (equal (romacs-agent--read-selection nil) '("claude")))
      (should-not (car asked))
      ;; C-u shows the palette anyway.
      (romacs-agent--read-selection '(4))
      (should (car asked)))))

(ert-deftest romacs-agent-picker-instance-effort-only-where-it-fits ()
  (romacs-agent-picker--with
      ((romacs-agent-providers `(("claude" :config-dir ,root :program ,program :model "a" :effort "xhigh")))
       (romacs-agent-models '((claude ("a" :efforts ("low" "xhigh")) ("b" :efforts ("low")))))
       (romacs-agent--sessions (make-hash-table :test #'equal))
       (made nil))
    (cl-letf (((symbol-function 'romacs-agent--create)
               (lambda (&rest fields) (setq made (plist-get fields :options)) (apply #'romacs-agent--make fields)))
              ((symbol-function 'romacs-agent--launch) #'ignore)
              ((symbol-function 'romacs-agent--attach) #'ignore)
              ((symbol-function 'romacs-agent--when-attached) #'ignore)
              ((symbol-function 'romacs-agent--argv) #'ignore))
      (romacs-agent-start root nil nil "claude" nil)
      (should (equal (plist-get made :model) "a"))
      (should (equal (plist-get made :effort) "xhigh"))
      ;; A model that does not take it is not started with it.
      (romacs-agent-start root nil nil "claude" '(:model "b"))
      (should (equal (plist-get made :model) "b"))
      (should-not (plist-get made :effort))
      ;; One the catalogue does not know: the user's word stands.
      (romacs-agent-start root nil nil "claude" '(:model "z"))
      (should (equal (plist-get made :effort) "xhigh")))))

(ert-deftest romacs-agent-picker-new-offers-the-exact-directory ()
  ;; A section or session in a repository's subdirectory: that directory,
  ;; not the repository's root.
  (romacs-agent-picker--with
      ((sub (file-name-as-directory (expand-file-name "repository/sub" root)))
       (offered nil) (started nil))
    (cl-letf (((symbol-function 'romacs-agent--project-default) (lambda () (expand-file-name "repository/" root)))
              ((symbol-function 'read-directory-name) (lambda (_prompt directory) (setq offered directory)))
              ((symbol-function 'romacs-agent--read-selection) (lambda (_ask) '("claude")))
              ((symbol-function 'read-string) (lambda (_prompt) ""))
              ((symbol-function 'romacs-agent-start)
               (lambda (&rest _) (interactive) (setq started (called-interactively-p 'any)))))
      (with-temp-buffer
        (insert (propertize "row\n" 'romacs-agents-project (directory-file-name sub)))
        (goto-char (point-min))
        (romacs-agents-new)
        (should (equal offered sub))
        (should started)
        ;; Off every section: the current project's.
        (goto-char (point-max))
        (romacs-agents-new)
        (should (equal offered (expand-file-name "repository/" root))))
      (let ((romacs-agent-ui--session (romacs-agent--make :id "s" :cwd (directory-file-name sub))))
        (setq offered nil)
        (romacs-agent-chat-new)
        (should (equal offered sub))))))

(ert-deftest romacs-agent-picker-capabilities-list-models ()
  (romacs-agent-picker--with
      ((romacs-agent-providers `(("codex" :config-dir ,root :program ,program)))
       (romacs-agent-models '((codex ("c" :label "C" :efforts ("low" "high") :default-effort "low") ("d")))))
    (cl-letf (((symbol-function 'romacs-mcp--instance) #'ignore))
      (let* ((provider (aref (gethash "providers" (romacs-agent-tools--capabilities (romacs-mcp--object))) 0))
             (models (gethash "models" provider))
             (option (aref (gethash "options" (aref models 0)) 0)))
        (should (equal (gethash "id" (aref models 0)) "c"))
        (should (equal (gethash "id" option) "reasoningEffort"))
        (should (equal (gethash "values" option) ["low" "high"]))
        (should (equal (gethash "default" option) "low"))
        (should (equal (gethash "label" (aref models 1)) "d"))
        (should (equal (gethash "options" (aref models 1)) []))))))

(provide 'romacs-agent-picker-tests)
;;; romacs-agent-picker-tests.el ends here
