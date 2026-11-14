;;; romacs-mcp-dogfood-tests.el --- What an agent using Romacs's tools sees -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-history)
  (require 'romacs-agent-t3)
  (require 'romacs-mcp-session))

;; These tests start no supervisor or harness: the build's own (beside the
;; binary) are not needed to make a session.
(advice-add 'romacs-agent--check-companions :override #'ignore)

;;; The user's own errors

(ert-deftest romacs-mcp-dogfood-user-errors-apart ()
  (dolist (data '((beginning-of-buffer) (end-of-buffer) (quit) (minibuffer-quit) (user-error "No more")
                  (text-read-only) (buffer-read-only "x") (mark-inactive)))
    (should (romacs-usage-log-user-error-p data)))
  (dolist (data '((error "Real fault") (wrong-type-argument listp 1) (void-function nosuch) (file-missing "x")))
    (should-not (romacs-usage-log-user-error-p data)))
  ;; The session's last error is the last fault; the user's own apart.
  (let ((romacs-mcp-session--last-error nil) (romacs-mcp-session--last-user-error nil))
    (romacs-mcp-session--command-error '(wrong-type-argument listp 1) nil nil)
    (romacs-mcp-session--command-error '(beginning-of-buffer) nil nil)
    (should (string-match-p "wrong-type-argument" (gethash "data" romacs-mcp-session--last-error)))
    (should (string-match-p "beginning-of-buffer" (gethash "data" romacs-mcp-session--last-user-error))))
  ;; The usage log tags them, so its readers share the one definition.
  (let* ((file (make-temp-file "romacs-usage-user-" nil ".jsonl"))
         (romacs-usage-log-file file) (romacs-usage-log--pending nil) (romacs-usage-log--capture nil)
         (real-this-command 'previous-line))
    (unwind-protect
        (progn
          (romacs-usage-log--command-error '(beginning-of-buffer) nil nil)
          (romacs-usage-log--command-error '(error "Real fault") nil nil)
          (romacs-usage-log-flush)
          (let ((records (with-temp-buffer
                           (insert-file-contents file)
                           (mapcar (lambda (line) (json-parse-string line :object-type 'plist))
                                   (split-string (buffer-string) "\n" t)))))
            (should (eq (plist-get (nth 0 records) :user) t))
            (should-not (plist-member (nth 1 records) :user))))
      (delete-file file))))

;;; Every recorder of a command error runs once around one report

(ert-deftest romacs-mcp-dogfood-command-error-recorders-compose ()
  ;; The session tools enabled again after the usage log advised the report
  ;; once made the report call itself, until `max-lisp-eval-depth' ended the
  ;; command: each recorder now runs once, and the report once.
  (let* ((reports 0) (logged 0) (recorded 0)
         (command-error-function (lambda (&rest _) (cl-incf reports)))
         (romacs-mcp-session--error-count 0)
         (romacs-mcp-session--last-error nil) (romacs-mcp-session--last-user-error nil))
    (cl-letf (((symbol-function 'romacs-usage-log--command-error) (lambda (&rest _) (cl-incf logged))))
      (romacs-mcp-enable-session-tools)
      (add-function :before command-error-function #'romacs-usage-log--command-error)
      (romacs-mcp-enable-session-tools)
      (funcall command-error-function '(end-of-buffer) nil nil)
      (setq recorded romacs-mcp-session--error-count))
    (should (equal (list reports logged recorded) '(1 1 1)))
    (should (string-match-p "end-of-buffer" (gethash "data" romacs-mcp-session--last-user-error)))))

;;; What observe lists

(ert-deftest romacs-mcp-dogfood-observe-buffers ()
  (let ((made (list (get-buffer-create "*agent: a private chat*")
                    (get-buffer-create "*-jabber-chat-someone@example.org*")
                    (get-buffer-create "*notmuch-search-inbox*")
                    (get-buffer-create "plain-notes"))))
    (unwind-protect
        (let ((summary (romacs-mcp-session--buffers (romacs-mcp--object))))
          ;; By default only how many of each kind.
          (should (hash-table-p summary))
          (let ((kinds (gethash "byKind" summary)))
            (should (>= (gethash "agent" kinds 0) 1))
            (should (>= (gethash "chat" kinds 0) 1))
            (should (>= (gethash "mail" kinds 0) 1)))
          (should-not (string-search "someone@example.org" (json-serialize summary)))
          ;; Names when asked; conversations only when asked for.
          (let ((names (append (romacs-mcp-session--buffers (romacs-mcp--object "buffers" t)) nil)))
            (should (member "plain-notes" names))
            (should-not (cl-some (lambda (n) (member n names))
                                 '("*agent: a private chat*" "*-jabber-chat-someone@example.org*" "*notmuch-search-inbox*"))))
          (let ((names (append (romacs-mcp-session--buffers (romacs-mcp--object "buffers" t "private" t)) nil)))
            (should (member "*-jabber-chat-someone@example.org*" names)))
          (should (equal (append (romacs-mcp-session--buffers (romacs-mcp--object "bufferKind" "agent")) nil)
                         '("*agent: a private chat*")))
          (should (equal (append (romacs-mcp-session--buffers (romacs-mcp--object "bufferRegexp" "\\`plain-")) nil)
                         '("plain-notes"))))
      (mapc #'kill-buffer made))))

(ert-deftest romacs-mcp-dogfood-help-and-instructions ()
  (let ((romacs-mcp-tools (copy-alist romacs-mcp-tools)))
    (romacs-mcp-register-tool "romacs_task_status" "A task's status. More words." (romacs-mcp--schema nil nil) #'ignore)
    (romacs-mcp-register-tool "zz_tool" (lambda () "Made now. More.") (romacs-mcp--schema nil nil) #'ignore)
    (let ((help (romacs-mcp--help nil)))
      (should (string-prefix-p "core:" help))
      (should (string-match-p "^  romacs_eval  " help))
      (should (string-match-p "^agents:\n  romacs_task_status  A task's status$" help))
      (should (string-match-p "^other:\n  zz_tool  Made now$" help))))
  (dolist (name '("romacs_identity" "romacs_observe" "romacs_eval" "romacs_help"))
    (should (string-search name romacs-mcp-instructions)))
  (should (< (length romacs-mcp-instructions) 800)))

;;; Titles

(defun romacs-mcp-dogfood--t3-store (file threads &optional title-column)
  "A T3 thread store FILE holding THREADS, (INSTANCE NATIVE TITLE UPDATED)
each; TITLE-COLUMN names the title's column (another schema's)."
  (let ((db (sqlite-open file)) (column (or title-column "title")))
    (sqlite-execute db (format "create table orchestration_v2_projection_threads (thread_id text, %s text)" column))
    (sqlite-execute db "create table orchestration_v2_projection_provider_threads (provider_thread_id text, thread_id text, driver text, payload_json text, updated_at text, provider_instance_id text)")
    (cl-loop for (instance native title updated) in threads for i from 0
             do (sqlite-execute db "insert into orchestration_v2_projection_threads values (?, ?)" (list (format "thread-%d" i) title))
             (sqlite-execute db "insert into orchestration_v2_projection_provider_threads values (?, ?, 'claudeAgent', ?, ?, ?)"
                             (list (format "p-%d" i) (format "thread-%d" i)
                                   (json-serialize (romacs-mcp--object "nativeThreadRef" (romacs-mcp--object "nativeId" native)))
                                   updated instance)))
    (sqlite-close db)))

(defun romacs-mcp-dogfood--transcript (home id prompt)
  (let ((dir (expand-file-name "projects/-work" home)))
    (make-directory dir t)
    (with-temp-file (expand-file-name (concat id ".jsonl") dir)
      (insert (json-serialize (romacs-mcp--object "type" "user" "cwd" "/work"
                                                  "message" (romacs-mcp--object "role" "user" "content" prompt)))
              "\n"))))

(defmacro romacs-mcp-dogfood--titles (&rest body)
  "BODY in a private home `root', T3's settings in ROOT/t3."
  `(let* ((root (make-temp-file "romacs-dogfood-titles-" t))
          (t3 (expand-file-name "t3" root))
          (romacs-agent-t3-settings (expand-file-name "settings.json" t3))
          (romacs-agent-t3--read nil) (romacs-agent-t3--titles (make-hash-table :test #'equal))
          (romacs-agent-history--descriptions (make-hash-table :test #'equal))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          ;; Its own home: the harnesses' default directories are under it.
          (process-environment (cons (concat "HOME=" root) process-environment)))
     (make-directory t3 t)
     (unwind-protect (progn ,@body) (delete-directory root t))))

(defun romacs-mcp-dogfood--listed-titles ()
  (romacs-agent-history-list)
  (sort (mapcar (lambda (i) (plist-get i :title)) (romacs-agent-history-items)) #'string<))

(defconst romacs-mcp-dogfood--id "11111111-2222-4333-8444-555555555555")
(defconst romacs-mcp-dogfood--other "11111111-2222-4333-8444-666666666666")

(ert-deftest romacs-mcp-dogfood-task-lines ()
  (should (equal (romacs-agent--task-line "Act as the implementation sub-agent for this task.\n\nFix the parser\nmore")
                 "Fix the parser"))
  (should (equal (romacs-agent--task-line "Act as the review sub-agent for this task.") "Act as the review sub-agent for this task."))
  (should (equal (romacs-agent--task-line "  plain first line\nsecond") "plain first line")))

(ert-deftest romacs-mcp-dogfood-titles-from-t3 ()
  (romacs-mcp-dogfood--titles
   (let ((claude (expand-file-name "claude" root)))
     (romacs-mcp-dogfood--transcript claude romacs-mcp-dogfood--id "Act as the implementation sub-agent for this task.\n\nFix the parser")
     (romacs-mcp-dogfood--transcript claude romacs-mcp-dogfood--other "Act as the research sub-agent for this task.\n\nMeasure closures")
     (with-temp-file romacs-agent-t3-settings
       (insert (format "{\"providerInstances\":{\"claudeAgent\":{\"driver\":\"claudeAgent\",\"config\":{\"homePath\":%S}}}}" claude)))
     ;; Its newest row of a thread wins; another instance's never.
     (romacs-mcp-dogfood--t3-store (romacs-agent-t3-store romacs-agent-t3-settings)
                                   (list (list "claudeAgent" romacs-mcp-dogfood--id "An older title" "2026-01-01")
                                         (list "claudeAgent" romacs-mcp-dogfood--id "Parser: the title T3 gave it" "2026-02-01")
                                         (list "claudeAgent_b" romacs-mcp-dogfood--other "Another instance's thread" "2026-03-01")))
     (let ((romacs-agent-providers '(t3)))
       (should (equal (romacs-mcp-dogfood--listed-titles) '("Measure closures" "Parser: the title T3 gave it"))))
     ;; Not T3's instance: the task's first line, its preamble skipped.
     (let ((romacs-agent-providers (list (cons "claude" claude))))
       (should (equal (romacs-mcp-dogfood--listed-titles) '("Fix the parser" "Measure closures")))))))

(ert-deftest romacs-mcp-dogfood-titles-per-instance-and-store ()
  ;; Two T3 instances' homes hold the same conversation id: each its own
  ;; title, from the store beside the settings that configure them, not
  ;; the default settings' store.
  (romacs-mcp-dogfood--titles
   (let* ((a (expand-file-name "a" root)) (b (expand-file-name "b" root))
          (explicit (expand-file-name "elsewhere/settings.json" root)))
     (make-directory (file-name-directory explicit) t)
     (romacs-mcp-dogfood--transcript a romacs-mcp-dogfood--id "Act as the implementation sub-agent for this task.\n\nIn a")
     (romacs-mcp-dogfood--transcript b romacs-mcp-dogfood--id "Act as the implementation sub-agent for this task.\n\nIn b")
     (with-temp-file explicit
       (insert (format "{\"providerInstances\":{\"claudeAgent_a\":{\"driver\":\"claudeAgent\",\"displayName\":\"A\",\"config\":{\"homePath\":%S}},\"claudeAgent_b\":{\"driver\":\"claudeAgent\",\"displayName\":\"B\",\"config\":{\"homePath\":%S}}}}" a b)))
     (romacs-mcp-dogfood--t3-store (romacs-agent-t3-store explicit)
                                   (list (list "claudeAgent_a" romacs-mcp-dogfood--id "A's own title" "1")
                                         (list "claudeAgent_b" romacs-mcp-dogfood--id "B's own title" "1")))
     (romacs-mcp-dogfood--t3-store (romacs-agent-t3-store romacs-agent-t3-settings)
                                   (list (list "claudeAgent_a" romacs-mcp-dogfood--id "The default store's" "9")))
     (let ((romacs-agent-providers (list (list 't3 :settings explicit))))
       (should (equal (romacs-mcp-dogfood--listed-titles) '("A's own title" "B's own title")))))))

(ert-deftest romacs-mcp-dogfood-titles-fall-back-quietly ()
  (romacs-mcp-dogfood--titles
   (let ((claude (expand-file-name "claude" root)) (messages nil))
     (romacs-mcp-dogfood--transcript claude romacs-mcp-dogfood--id "Act as the implementation sub-agent for this task.\n\nFix the parser")
     (with-temp-file romacs-agent-t3-settings
       (insert (format "{\"providerInstances\":{\"claudeAgent\":{\"driver\":\"claudeAgent\",\"config\":{\"homePath\":%S}}}}" claude)))
     (cl-letf (((symbol-function 'message) (lambda (&rest args) (push args messages))))
       (let ((romacs-agent-providers '(t3)))
         ;; No store: the prompt's title.
         (should (equal (romacs-mcp-dogfood--listed-titles) '("Fix the parser")))
         ;; Another schema (a renamed column): the same.
         (romacs-mcp-dogfood--t3-store (romacs-agent-t3-store romacs-agent-t3-settings)
                                       (list (list "claudeAgent" romacs-mcp-dogfood--id "Parser: T3's title" "1"))
                                       "name")
         (should (equal (romacs-mcp-dogfood--listed-titles) '("Fix the parser")))))
     (should-not messages))))

(ert-deftest romacs-mcp-dogfood-titles-failure-not-kept ()
  ;; A read that fails (a lock) is tried again at the next refresh, the
  ;; store's file unchanged; one that worked is not read again.
  (romacs-mcp-dogfood--titles
   (let ((store (romacs-agent-t3-store romacs-agent-t3-settings)) (reads 0) (fail t))
     (romacs-mcp-dogfood--t3-store store (list (list "claudeAgent" romacs-mcp-dogfood--id "Parser" "1")))
     (cl-letf* ((select (symbol-function 'sqlite-select))
                ((symbol-function 'sqlite-select)
                 (lambda (&rest args) (cl-incf reads)
                   (if fail (signal 'sqlite-error '("database is locked")) (apply select args)))))
       (should-not (romacs-agent-t3-thread-titles store))
       (setq fail nil)
       (let ((titles (romacs-agent-t3-thread-titles store)))
         (should (equal (gethash (romacs-agent-t3-title-key "claudeAgent" romacs-mcp-dogfood--id) titles) "Parser")))
       (romacs-agent-t3-thread-titles store)
       (should (= reads 2))))))

(ert-deftest romacs-mcp-dogfood-titles-stepping-error-not-kept ()
  ;; A row SQLite cannot step past (malformed JSON in its payload) fails
  ;; the read: no titles, nothing kept for the next refresh.
  (romacs-mcp-dogfood--titles
   (let ((store (romacs-agent-t3-store romacs-agent-t3-settings)))
     (romacs-mcp-dogfood--t3-store store (list (list "claudeAgent" romacs-mcp-dogfood--id "Fine" "1")
                                               (list "claudeAgent" romacs-mcp-dogfood--other "Broken" "2")))
     (let ((db (sqlite-open store)))
       (sqlite-execute db "update orchestration_v2_projection_provider_threads set payload_json = '{not json' where updated_at = '2'")
       (sqlite-close db))
     (should-not (romacs-agent-t3-thread-titles store))
     (should-not (gethash store romacs-agent-t3--titles)))))

(ert-deftest romacs-mcp-dogfood-t3-titles-as-t3-shows-them ()
  ;; T3's title is not a prompt: one starting with < is kept.
  (romacs-mcp-dogfood--titles
   (let ((claude (expand-file-name "claude" root)))
     (romacs-mcp-dogfood--transcript claude romacs-mcp-dogfood--id "Act as the implementation sub-agent for this task.\n\nFix it")
     (with-temp-file romacs-agent-t3-settings
       (insert (format "{\"providerInstances\":{\"claudeAgent\":{\"driver\":\"claudeAgent\",\"config\":{\"homePath\":%S}}}}" claude)))
     (romacs-mcp-dogfood--t3-store (romacs-agent-t3-store romacs-agent-t3-settings)
                                   (list (list "claudeAgent" romacs-mcp-dogfood--id "<button> click handler" "1")))
     (let ((romacs-agent-providers '(t3)))
       (should (equal (romacs-mcp-dogfood--listed-titles) '("<button> click handler")))))))

;;; Delegation runs on the delegating session's own instance

(ert-deftest romacs-mcp-dogfood-delegation-instance ()
  (let* ((root (make-temp-file "romacs-dogfood-delegate-" t))
         (romacs-agent-providers (list (cons "claude" (expand-file-name "claude" root))
                                       (cons "codex" (expand-file-name "codex" root))))
         (romacs-agent-default-provider "codex")
         (romacs-agent--sessions (make-hash-table :test #'equal))
         ;; Sessions' directories go under ROOT, not the user's state.
         (process-environment (cons (concat "XDG_STATE_HOME=" root) process-environment))
         (create (symbol-function 'romacs-agent--create))
         (started nil))
    (make-directory (expand-file-name "claude" root)) (make-directory (expand-file-name "codex" root))
    (puthash "parent" (romacs-agent--make :id "parent" :harness 'claude :provider "claude" :cwd root) romacs-agent--sessions)
    (unwind-protect
        (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () "parent"))
                  ((symbol-function 'romacs-mcp--instance) #'ignore)
                  ((symbol-function 'romacs-agent-tools--task) (lambda (s) s))
                  ((symbol-function 'romacs-agent--create)
                   (lambda (&rest fields) (push fields started) (apply create fields)))
                  ((symbol-function 'romacs-agent--launch) #'ignore)
                  ((symbol-function 'romacs-agent--attach) #'ignore)
                  ((symbol-function 'romacs-agent--manifest) #'ignore))
          ;; No target: the parent's own instance, not the default.
          (romacs-agent-tools--delegate (romacs-mcp--object "task" "say hi"))
          (should (equal (plist-get (car started) :provider) "claude"))
          ;; Named: that one.
          (romacs-agent-tools--delegate (romacs-mcp--object "task" "say hi" "target" (romacs-mcp--object "provider" "codex")))
          (should (equal (plist-get (car started) :provider) "codex"))
          ;; Unknown: refused, naming what there is.
          (let ((err (should-error (romacs-agent-tools--delegate
                                    (romacs-mcp--object "task" "x" "target" (romacs-mcp--object "provider" "claude9")))
                                   :type 'user-error)))
            (should (string-match-p "claude9.*configured: claude, codex" (cadr err))))
          ;; The description names them.
          (should (string-match-p "your own session's instance.*claude, codex" (romacs-agent-tools--delegate-description))))
      (delete-directory root t))))

(provide 'romacs-mcp-dogfood-tests)
;;; romacs-mcp-dogfood-tests.el ends here
