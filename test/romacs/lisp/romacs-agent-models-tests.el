;;; romacs-agent-models-tests.el --- The models each harness reports -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent)
  (require 'romacs-agent-claude)
  (require 'romacs-agent-codex)
  (require 'romacs-agent-menu))

(defmacro romacs-agent-models-tests--with (script bindings &rest body)
  "BODY in a private state directory, `root' its root and `program' an
executable there running the shell SCRIPT; BINDINGS, of `let*', see them."
  (declare (indent 2))
  `(let* ((root (make-temp-file "romacs-models-" t))
          (program (expand-file-name "fake-codex" root))
          (process-environment (cons (concat "XDG_STATE_HOME=" root) process-environment))
          (romacs-agent--models-probes (make-hash-table :test #'equal))
          ,@bindings)
     (with-temp-file program (insert "#!/bin/sh\n" ,script))
     (set-file-modes program #o755)
     (unwind-protect (progn ,@body)
       (delete-directory root t))))

(defconst romacs-agent-models-tests--listed
  (concat "{\"id\":\"model/list:2\",\"result\":{\"nextCursor\":null,\"data\":["
          "{\"id\":\"gpt-6.2-sol\",\"model\":\"gpt-6.2-sol\",\"displayName\":\"GPT-6.2-Sol\",\"isDefault\":true,"
          "\"hidden\":false,\"defaultReasoningEffort\":\"low\",\"supportedReasoningEfforts\":"
          "[{\"reasoningEffort\":\"low\",\"description\":\"\"},{\"reasoningEffort\":\"ultra\",\"description\":\"\"}]},"
          "{\"id\":\"gpt-6.2-astra\",\"model\":\"gpt-6.2-astra\",\"displayName\":\"GPT-6.2-Astra\",\"isDefault\":false,"
          "\"hidden\":false,\"defaultReasoningEffort\":\"medium\",\"supportedReasoningEfforts\":"
          "[{\"reasoningEffort\":\"medium\",\"description\":\"\"}]}]}}")
  "What a Codex app-server answers `model/list', as codex-cli 0.171 does.")

(defun romacs-agent-models-tests--json (text)
  (json-parse-string text :object-type 'hash-table :null-object :null :false-object :false))

(defun romacs-agent-models-tests--probed (instance)
  "Probe INSTANCE, waiting for its outcome."
  (let ((outcome nil))
    (should (romacs-agent--probe-models instance (lambda (o) (setq outcome o))))
    (with-timeout (10 (error "No outcome"))
      (while (not outcome) (accept-process-output nil 0.05)))
    outcome))

(ert-deftest romacs-agent-models-the-table-ships-no-model-ids ()
  ;; Each harness reports its own: a list shipped here only goes stale.
  (should-not (eval (car (get 'romacs-agent-models 'standard-value)) t)))

(ert-deftest romacs-agent-models-codex-model-list-read ()
  (let ((line (romacs-agent-models-tests--json romacs-agent-models-tests--listed)))
    (should (equal (romacs-agent--codex-probed-models line)
                   '(:models (("gpt-6.2-sol" :label "GPT-6.2-Sol" :efforts ("low" "ultra") :default-effort "low")
                              ("gpt-6.2-astra" :label "GPT-6.2-Astra" :efforts ("medium")
                               :default-effort "medium"))))))
  ;; Another line is no answer; a refusal says why.
  (should-not (romacs-agent--codex-probed-models (romacs-agent-models-tests--json "{\"method\":\"x\"}")))
  (should-not (romacs-agent--codex-probed-models (romacs-agent-models-tests--json "{\"id\":\"initialize:1\",\"result\":{}}")))
  (should (equal (romacs-agent--codex-probed-models
                  (romacs-agent-models-tests--json "{\"id\":\"initialize:1\",\"error\":{\"message\":\"no\"}}"))
                 '(:error "no"))))

(ert-deftest romacs-agent-models-claude-probe-read ()
  (let ((answer (lambda (id subtype)
                  (romacs-agent-models-tests--json
                   (format "{\"type\":\"control_response\",\"response\":{\"subtype\":%S,\"request_id\":%S,\"response\":{\"models\":[%s]}}}"
                           subtype id "{\"value\":\"fable\",\"resolvedModel\":\"claude-fable-5-2\",\"displayName\":\"Fable 5.2\",\"supportedEffortLevels\":[\"low\"]}")))))
    (should (equal (romacs-agent--claude-probed-models (funcall answer "romacs-models" "success"))
                   '(:models (("claude-fable-5-2" :label "Fable 5.2" :efforts ("low"))))))
    (should-not (romacs-agent--claude-probed-models (funcall answer "romacs-init-1" "success")))
    (should (eq (car (romacs-agent--claude-probed-models (funcall answer "romacs-models" "error"))) :error))))

(ert-deftest romacs-agent-models-a-probe-keeps-what-its-harness-reports ()
  ;; The real shape: a program that reads its requests, answers, and runs
  ;; on until it is stopped.  The answer is kept, the program stopped.
  (romacs-agent-models-tests--with
      (format "read a; read b; read c\necho '{\"id\":\"initialize:1\",\"result\":{}}'\necho '%s'\nexec sleep 60\n"
              romacs-agent-models-tests--listed)
      ((romacs-agent-providers `(("codex" :program ,program)))
       (romacs-agent-models '((codex ("gpt-6.1-sol" :label "Stale") ("gpt-6.2-sol" :label "Mine")))))
    (let ((instance (romacs-agent--configured "codex")))
      (should (equal (car (romacs-agent-models-tests--probed instance)) :models))
      (should-not (process-live-p (gethash "codex" romacs-agent--models-probes)))
      (should (eq (gethash "codex" romacs-agent--models-probes) 'done))
      (let ((rows (romacs-agent--models instance)))
        ;; What the harness reports goes first and wins over the table.
        (should (equal (mapcar #'car rows) '("gpt-6.2-sol" "gpt-6.2-astra" "gpt-6.1-sol")))
        (should (equal (plist-get (cdr (assoc "gpt-6.2-sol" rows)) :label) "GPT-6.2-Sol"))
        ;; Its default effort is kept with it.
        (should (equal (plist-get (cdr (assoc "gpt-6.2-astra" rows)) :default-effort) "medium")))
      ;; Kept for this program only: another one's cache does not hold.
      (set-file-times program (time-add (current-time) 100))
      (should (equal (mapcar #'car (romacs-agent--models instance)) '("gpt-6.1-sol" "gpt-6.2-sol"))))))

(ert-deftest romacs-agent-models-a-probe-unanswered-is-stopped ()
  (romacs-agent-models-tests--with "exec sleep 60\n"
      ((romacs-agent-providers `(("codex" :program ,program)))
       (romacs-agent--probe-timeout 0.2))
    (should (equal (romacs-agent-models-tests--probed (romacs-agent--configured "codex")) '(:error "no answer")))
    (should-not (file-exists-p (romacs-agent--models-file "codex"))))
  ;; One that ends without answering says so.
  (romacs-agent-models-tests--with "exit 1\n" ((romacs-agent-providers `(("codex" :program ,program))))
    (should (eq (car (romacs-agent-models-tests--probed (romacs-agent--configured "codex"))) :error))))

(ert-deftest romacs-agent-models-claude-probe-takes-no-session-s-arguments ()
  ;; An instance's arguments may name settings and hooks: a probe runs
  ;; none of them, and none of the user's settings.
  (let ((argv (plist-get (romacs-agent--claude-models-probe
                          (list :program "claude" :arguments '("--settings" "hooks.json")))
                         :argv)))
    (should-not (member "--settings" argv))
    (should (equal (member "--setting-sources" argv) '("--setting-sources" "")))
    (should (member "--no-session-persistence" argv))))

(ert-deftest romacs-agent-models-an-empty-answer-is-kept ()
  ;; A harness that now reports no model: what it offered before goes.
  (romacs-agent-models-tests--with "exit 0\n" ((romacs-agent-providers `(("codex" :program ,program))))
    (let ((instance (romacs-agent--configured "codex")))
      (romacs-agent--keep-models instance '(("a" :label "A")))
      (should (equal (mapcar #'car (romacs-agent--models instance)) '("a")))
      (romacs-agent--keep-models instance nil)
      (should-not (romacs-agent--models instance)))))

(ert-deftest romacs-agent-models-asked-once-a-run ()
  (let ((asked (make-hash-table :test #'equal))
        (instances (list (list :name "codex") (list :name "claude"))))
    (should (equal (romacs-agent--models-due instances asked) instances))
    (puthash "codex" 'done asked)
    (should (equal (romacs-agent--models-due instances asked) (list (list :name "claude"))))))

(ert-deftest romacs-agent-models-the-menu-offers-the-reported-models ()
  (romacs-agent-models-tests--with "exit 0\n"
      ((romacs-agent-providers `(("codex" :program ,program :enabled nil)))
       (session (romacs-agent--make :id "s" :harness 'codex :provider "codex"))
       (offered nil) (wanted nil))
    (romacs-agent--keep-models (romacs-agent--configured "codex")
                               (cadr (romacs-agent--codex-probed-models
                                      (romacs-agent-models-tests--json romacs-agent-models-tests--listed))))
    (cl-letf (((symbol-function 'completing-read)
               (lambda (_prompt table &rest _) (setq offered (all-completions "" table)) (car offered)))
              ((symbol-function 'romacs-agent--want-models)
               (lambda (&optional instances) (setq wanted (append wanted (mapcar (lambda (i) (plist-get i :name)) instances)))))
              ((symbol-function 'romacs-agent-select) #'ignore)
              ((symbol-function 'message) #'ignore))
      (with-temp-buffer
        (setq-local romacs-agent-log--session session)
        (call-interactively #'romacs-agent-chat-model)))
    ;; Its harness asked, in the background, as the menu opened: its own
    ;; instance's, a session resumed under one disabled since too.
    (should (member "codex" wanted))
    (should (member "Codex · GPT-6.2-Sol" offered))
    (should (member "Codex · GPT-6.2-Astra" offered))))

(provide 'romacs-agent-models-tests)
;;; romacs-agent-models-tests.el ends here
