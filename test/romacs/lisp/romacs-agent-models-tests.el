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
                   '(:models (("gpt-6.2-sol" :label "GPT-6.2-Sol" :default-model t :efforts ("low" "ultra") :default-effort "low")
                              ("gpt-6.2-astra" :label "GPT-6.2-Astra" :default-model nil :efforts ("medium")
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
      (should (numberp (gethash "codex" romacs-agent--models-probes)))
      (let ((rows (romacs-agent--models instance)))
        ;; What the harness reports goes first and wins over the table.
        (should (equal (mapcar #'car rows) '("gpt-6.2-sol" "gpt-6.2-astra" "gpt-6.1-sol")))
        (should (equal (plist-get (cdr (assoc "gpt-6.2-sol" rows)) :label) "GPT-6.2-Sol"))
        ;; Its default effort is kept with it.
        (should (equal (plist-get (cdr (assoc "gpt-6.2-astra" rows)) :default-effort) "medium")))
      ;; Kept for this program only: another one's does not hold.
      (let ((other (expand-file-name "other-codex" root)))
        (copy-file program other)
        (should (equal (mapcar #'car (romacs-agent--models (plist-put (copy-sequence instance) :program other)))
                       '("gpt-6.1-sol" "gpt-6.2-sol")))))))

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
  ;; An instance's arguments may name servers or hooks: a probe takes
  ;; none of them, and runs none of the user's hooks.
  (let ((argv (plist-get (romacs-agent--claude-models-probe
                          (list :program "claude" :arguments '("--mcp-config" "servers.json")))
                         :argv)))
    (should-not (member "servers.json" argv))
    (should (equal (member "--settings" argv) '("--settings" "{\"disableAllHooks\":true}")))
    (should (member "--no-session-persistence" argv)))
  ;; Arguments naming settings of their own: their sessions alone report.
  (dolist (arguments '(("--settings" "s.json") ("--settings=s.json") ("--setting-sources=user")
                       ("--setting-sources" "user")))
    (should-not (romacs-agent--claude-models-probe (list :program "claude" :arguments arguments))))
  (should (romacs-agent--claude-models-probe (list :program "claude" :arguments '("--settingsx")))))

(ert-deftest romacs-agent-models-an-empty-answer-is-kept ()
  ;; A harness that now reports no model: what it offered before goes.
  (romacs-agent-models-tests--with "exit 0\n" ((romacs-agent-providers `(("codex" :program ,program))))
    (let ((instance (romacs-agent--configured "codex")))
      (romacs-agent--keep-models instance '(("a" :label "A")))
      (should (equal (mapcar #'car (romacs-agent--models instance)) '("a")))
      (romacs-agent--keep-models instance nil)
      (should-not (romacs-agent--models instance)))))

;; Asked when nothing is kept for its program, once a run: sessions report
;; the rest, and `romacs-agent-refresh' asks again on demand.
(ert-deftest romacs-agent-models-asked-once-unless-kept ()
  (romacs-agent-models-tests--with "exit 0\n" ((romacs-agent-providers `(("codex" :program ,program))))
    (let* ((instance (romacs-agent--configured "codex")) (instances (list instance))
           (asked (make-hash-table :test #'equal)))
      ;; Nothing kept, not asked in this run: due.
      (should (equal (romacs-agent--models-due instances asked) instances))
      ;; Asked in this run, answered or not: not again.
      (puthash "codex" 1000 asked)
      (should-not (romacs-agent--models-due instances asked))
      ;; Kept for its program: not asked, in a new run either.
      (clrhash asked)
      (romacs-agent--keep-models instance '(("a" :label "A")))
      (clrhash asked)
      (should-not (romacs-agent--models-due instances asked))
      ;; Its program updated: asked again.
      (let ((updated (plist-put (copy-sequence instance) :program (expand-file-name "other-codex" root))))
        (copy-file program (plist-get updated :program))
        (should (equal (romacs-agent--models-due (list updated) asked) (list updated)))))))

(ert-deftest romacs-agent-models-no-timer-asks ()
  ;; Loading asks nothing on a schedule: a probe can renew a sign-in.
  (should-not (cl-find-if (lambda (timer)
                            (string-prefix-p "romacs-agent--models" (format "%s" (timer--function timer))))
                          timer-list)))

(defun romacs-agent-models-tests--claude-home (expires)
  "A Claude configuration directory whose sign-in EXPIRES then (seconds)."
  (let ((home (make-temp-file "claude-home" t)))
    (with-temp-file (expand-file-name ".credentials.json" home)
      (insert (format "{\"claudeAiOauth\":{\"accessToken\":\"t\",\"expiresAt\":%d}}"
                      (round (* 1000 expires)))))
    home))

(ert-deftest romacs-agent-models-claude-probe-renews-no-sign-in ()
  ;; A sign-in due to renew: not asked, as the probe would renew it under
  ;; its sessions; a fresh one, or none, is.
  (let ((due (romacs-agent-models-tests--claude-home (+ (float-time) 60)))
        (fresh (romacs-agent-models-tests--claude-home (+ (float-time) 3600)))
        (none (make-temp-file "claude-home" t)))
    (unwind-protect
        (progn
          (should (stringp (romacs-agent--claude-models-probe (list :program "claude" :config-dir due))))
          (should (consp (romacs-agent--claude-models-probe (list :program "claude" :config-dir fresh))))
          (should (consp (romacs-agent--claude-models-probe (list :program "claude" :config-dir none)))))
      (mapc (lambda (d) (delete-directory d t)) (list due fresh none)))))

(ert-deftest romacs-agent-models-a-declined-probe-stays-due ()
  ;; The driver says why not now: told, nothing run, asked again later.
  (romacs-agent-models-tests--with "exit 0\n" ((romacs-agent-providers `(("codex" :program ,program))))
    (let ((instance (romacs-agent--configured "codex")) (outcome nil))
      (cl-letf (((plist-get (romacs-agent--driver-of 'codex) :models-probe) (lambda (_) "not now")))
        (should-not (romacs-agent--probe-models instance (lambda (o) (setq outcome o)))))
      (should (equal outcome '(:error "not now")))
      (should (equal (romacs-agent--models-due (list instance) romacs-agent--models-probes) (list instance))))))

(ert-deftest romacs-agent-models-refresh-asks-every-account ()
  ;; Each enabled account: its usage by its harness's :usage, its models
  ;; by its probe; one answer list once every answer is in.
  (romacs-agent-models-tests--with
      (format "read a; read b; read c\necho '{\"id\":\"initialize:1\",\"result\":{}}'\necho '%s'\nexec sleep 60\n"
              romacs-agent-models-tests--listed)
      ((home (romacs-agent-models-tests--claude-home (+ (float-time) 60)))
       (romacs-agent-providers `(("codex" :program ,program) ("claude" :program ,program :config-dir ,home)))
       (noted nil) (results nil) (said nil))
    (cl-letf (((symbol-function 'romacs-agent-note-room) (lambda (name report) (push (cons name report) noted)))
              ((symbol-function 'message) (lambda (&rest args) (push (apply #'format-message args) said))))
      (romacs-agent-refresh (lambda (r) (setq results r)))
      (with-timeout (10 (error "No answer"))
        (while (not results) (accept-process-output nil 0.05)))
      (romacs-agent--refresh-say results))
    (unwind-protect
        (let ((codex (cdr (assoc "codex" results))) (claude (cdr (assoc "claude" results))))
          ;; Codex: no usage to ask; its models reported and kept.
          (should-not (plist-get codex :usage))
          (should (eq (car (plist-get codex :models)) :models))
          ;; Claude, its sign-in due to renew: neither asked, both say why.
          (should (equal (plist-get claude :usage) (list :error (romacs-agent--claude-renew-due home))))
          (should (equal (plist-get claude :models) (list :error (romacs-agent--claude-renew-due home))))
          (should-not noted)
          (should (string-match-p "codex: no usage to ask; 2 models" (car said))))
      (delete-directory home t))))

(ert-deftest romacs-agent-models-an-open-picker-shows-what-is-reported ()
  ;; The menu opened before Codex answered: its answer shows in the rows
  ;; still being asked for, the menu not opened again.
  (romacs-agent-models-tests--with "exit 0\n"
      ((romacs-agent-providers `(("codex" :program ,program)))
       (romacs-agent-models-changed-functions nil)
       (seen nil))
    (let ((instance (romacs-agent--configured "codex"))
          (minibuffer-setup-hook nil))
      (cl-letf (((symbol-function 'romacs-agent--want-models) #'ignore)
                ((symbol-function 'completing-read)
                 ;; In its minibuffer, what it completes is what it shows.
                 (lambda (_prompt candidates &rest _)
                   (with-temp-buffer
                     (setq-local minibuffer-completion-table candidates)
                     (run-hooks 'minibuffer-setup-hook)
                     (push (all-completions "" minibuffer-completion-table) seen)
                     (romacs-agent--keep-models
                      instance (cadr (romacs-agent--codex-probed-models
                                      (romacs-agent-models-tests--json romacs-agent-models-tests--listed))))
                     (push (all-completions "" minibuffer-completion-table) seen))
                   "Codex · GPT-6.2-Astra")))
        (should (equal (plist-get (cdr (romacs-agent--pick-row
                                         (lambda () (romacs-agent--instance-rows instance "Codex" nil))))
                                  :model)
                       "gpt-6.2-astra")))
      (should (equal (cadr seen) '("Codex · default")))
      (should (member "Codex · GPT-6.2-Sol" (car seen)))
      (should (equal (plist-get (cdar (romacs-agent--instance-rows instance "Codex" nil)) :note)
                     "gpt-6.2-sol · effort low"))
      ;; Its watch ends with it.
      (should-not romacs-agent-models-changed-functions))))

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
