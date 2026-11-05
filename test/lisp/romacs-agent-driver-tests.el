;;; romacs-agent-driver-tests.el --- Harness drivers and what their children inherit -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
(require 'ert)
(require 'cl-lib)
(require 'romacs-mcp)

;; Loading romacs-agent installs romacs-agent-tools: the endpoint's
;; registrations are left as they were.
(let ((romacs-mcp-call-function romacs-mcp-call-function)
      (romacs-mcp-tools (copy-alist romacs-mcp-tools)))
  (require 'romacs-agent))

(defmacro romacs-agent-driver-test--registry (&rest body)
  "BODY with the driver registry its own: what it defines goes with it."
  `(let ((romacs-agent--drivers (copy-alist romacs-agent--drivers))
         (romacs-agent--driver-table (make-hash-table :test #'eq)))
     (dolist (cell romacs-agent--drivers) (puthash (car cell) cell romacs-agent--driver-table))
     ,@body))

(ert-deftest romacs-agent-driver-registry ()
  (romacs-agent-driver-test--registry
    (romacs-agent-define-harness 'testdrv :lifetime 'per-turn :config-variable "TESTDRV_HOME"
                                 :config-default "~/.testdrv" :scrub '("TESTDRV_CHILD"))
    ;; The last defined first; defining it again replaces it.
    (should (eq (car (car romacs-agent--drivers)) 'testdrv))
    (romacs-agent-define-harness 'testdrv :lifetime 'persistent :config-variable "TESTDRV_HOME")
    (should (= (cl-count 'testdrv romacs-agent--drivers :key #'car) 1))
    (should (eq (plist-get (romacs-agent--driver-of 'testdrv) :lifetime) 'persistent))
    ;; Its variables join what no harness inherits.
    (should (member "TESTDRV_HOME" (romacs-agent--dropped-variables)))
    ;; A session looks its driver up once.
    (let ((session (romacs-agent--make :id "x" :harness 'testdrv)))
      (should-not (romacs-agent--per-turn-p session))
      (should (eq (romacs-agent-driver session) (romacs-agent--driver-of 'testdrv)))
      (setf (alist-get 'testdrv romacs-agent--drivers) '(:lifetime per-turn))
      (should-not (romacs-agent--per-turn-p session)))
    (should-error (romacs-agent--driver-of 'nosuch) :type 'user-error)))

(ert-deftest romacs-agent-import-ids-are-conversation-identities ()
  ;; The same conversation id in two configuration directories is two
  ;; conversations; a default spelled out is the default.
  (let ((a (romacs-agent--import-id 'claude "/home/x/.claude-a" "11111111-2222-4333-8444-555555555555"))
        (b (romacs-agent--import-id 'claude "/home/x/.claude-b" "11111111-2222-4333-8444-555555555555")))
    (should-not (equal a b))
    (should (string-match-p "\\`[0-9a-f]\\{8\\}-[0-9a-f]\\{4\\}-[0-9a-f]\\{4\\}-[0-9a-f]\\{4\\}-[0-9a-f]\\{12\\}\\'" a))
    (should (equal (romacs-agent--import-id 'codex nil "t") (romacs-agent--import-id 'codex "~/.codex" "t")))))

(defmacro romacs-agent-driver-test--launched (systemd &rest body)
  "BODY with `call-process' recording each call as (PROGRAM ARGS
ENVIRONMENT) in `calls'; SYSTEMD nil makes systemd-run fail (the
detached fallback)."
  (declare (indent 1))
  `(let* ((calls nil) (root (make-temp-file "romacs-agent-driver-" t))
          (process-environment
           (append (list "CLAUDECODE=1" "CLAUDE_CODE_SESSION_ID=parent" "CLAUDE_CONFIG_DIR=/parent/claude"
                         "T3CODE_THREAD_ID=thread" "CODEX_HOME=/parent/codex" "CODEX_THREAD_ID=parent"
                         "ROMACS_USAGE_LOG=/stale" "KEEP_ME=editor")
                   process-environment)))
     (unwind-protect
         (cl-letf (((symbol-function 'call-process)
                    (lambda (program _infile _destination _display &rest args)
                      (push (list program args (copy-sequence process-environment)) calls)
                      (if (and (equal program "systemd-run") (not ,systemd)) 1 0)))
                   ((symbol-function 'romacs-agent--route-here) #'ignore))
           ,@body)
       (delete-directory root t))))

(defun romacs-agent-driver-test--session (root harness config-dir)
  (let ((dir (expand-file-name (symbol-name harness) root)))
    (make-directory dir t)
    (romacs-agent--make :id "11111111-2222-4333-8444-555555555555" :harness harness :provider (symbol-name harness)
                        :config-dir config-dir :cwd root :dir dir :title "t"
                        :host-socket (expand-file-name (concat (symbol-name harness) "-run/host.sock") root))))

(defun romacs-agent-driver-test--check (calls harness config-dir)
  "The harness HARNESS's launch in CALLS: its environment holds no parent
harness's, orchestrator's or editor launch variable, and its command
line's `env' sets only its own configuration variable."
  (let* ((launch (cl-find-if (lambda (call) (member (car call) '("setsid" "systemd-run"))) calls :from-end nil))
         (environment (nth 2 launch))
         (args (nth 1 launch))
         (env-words (let ((tail (member "env" args))) (seq-take tail 3)))
         (variable (plist-get (romacs-agent--driver-of harness) :config-variable)))
    (dolist (name '("CLAUDECODE" "CLAUDE_CODE_SESSION_ID" "CLAUDE_CONFIG_DIR" "T3CODE_THREAD_ID"
                    "CODEX_HOME" "CODEX_THREAD_ID" "ROMACS_USAGE_LOG"))
      (should-not (let ((process-environment environment)) (getenv name))))
    (should (equal (let ((process-environment environment)) (getenv "KEEP_ME")) "editor"))
    (if config-dir
        (should (equal (seq-take env-words 2) (list "env" (concat variable "=" config-dir))))
      (should (equal env-words (list "env" "-u" variable))))
    launch))

(ert-deftest romacs-agent-children-inherit-no-harness-variables ()
  (dolist (systemd '(t nil))
    (dolist (case '((claude nil) (codex "/home/x/.codex-second")))
      (romacs-agent-driver-test--launched systemd
        (let* ((harness (car case)) (config-dir (cadr case))
               (session (romacs-agent-driver-test--session root harness config-dir)))
          (romacs-agent--launch session (romacs-agent--argv session nil))
          (let* ((launch (romacs-agent-driver-test--check calls harness config-dir))
                 (args (nth 1 launch)))
            (should (equal (car launch) (if systemd "systemd-run" "setsid")))
            (when systemd
              ;; The unit unsets them from the user manager's environment
              ;; too, and is given none of them by name.
              (let ((unset (cl-find-if (lambda (a) (string-prefix-p "--property=UnsetEnvironment=" a)) args)))
                (dolist (name '("CLAUDECODE" "CLAUDE_CONFIG_DIR" "T3CODE_THREAD_ID" "CODEX_HOME" "ROMACS_USAGE_LOG"))
                  (should (member name (split-string (substring unset (length "--property=UnsetEnvironment=")))))
                  (should-not (member (concat "--setenv=" name) args)))))))))))

(defconst romacs-agent-driver-test--fakerpc
  (expand-file-name "../../crates/romacs-vm-gui/harness/fakerpc.el"
                    (file-name-directory (or load-file-name buffer-file-name))))

(defun romacs-agent-driver-test--feed (session seq key object)
  "Journal record SEQ of SESSION: OBJECT under KEY (\"in\" or \"out\")."
  (let ((record (make-hash-table :test #'equal)))
    (puthash "seq" seq record)
    (puthash key (json-parse-string (json-serialize object) :object-type 'hash-table) record)
    (romacs-agent--record session record)))

(ert-deftest romacs-agent-startup-step-waits-for-its-own-echo ()
  ;; An interrupt written while a startup step is out (its echo first)
  ;; does not count as that step's: one session/new, not two.
  (romacs-agent-driver-test--registry
   (let ((romacs-agent--sessions (make-hash-table :test #'equal))
         (written nil))
    (load romacs-agent-driver-test--fakerpc nil t)
    (let ((session (romacs-agent--make :id "rpc-test" :harness 'fakerpc :cwd "/tmp")))
      (puthash "rpc-test" session romacs-agent--sessions)
      (cl-letf (((symbol-function 'romacs-agent--line) (lambda (_ object) (push object written)))
                ((symbol-function 'process-live-p) (lambda (_) t)))
        (setf (romacs-agent-caught-up session) t)
        (romacs-agent--step session)
        (let ((initialize (car written)))
          (should (equal (gethash "method" initialize) "initialize"))
          (romacs-agent-driver-test--feed session 1 "in" initialize)
          (romacs-agent-driver-test--feed session 2 "out" (romacs-mcp--object "jsonrpc" "2.0" "id" (gethash "id" initialize)
                                                                               "result" (romacs-mcp--object)))
          (should (equal (gethash "method" (car written)) "session/new"))
          ;; The interrupt written before session/new: its echo comes first.
          (romacs-agent-driver-test--feed session 3 "in" (romacs-mcp--object "jsonrpc" "2.0" "method" "session/cancel"))
          (romacs-agent-driver-test--feed session 4 "in" (car written))
          (romacs-agent-driver-test--feed session 5 "out" (romacs-mcp--object "jsonrpc" "2.0" "id" (gethash "id" (car written))
                                                                               "result" (romacs-mcp--object "sessionId" "rpc-x")))
          (should (= (cl-count "session/new" written :key (lambda (o) (gethash "method" o)) :test #'equal) 1))
          (should (romacs-agent--ready-p session))
          (should (equal (romacs-agent-harness-session session) "rpc-x"))))))))

(provide 'romacs-agent-driver-tests)
;;; romacs-agent-driver-tests.el ends here
