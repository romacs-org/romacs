;;; romacs-agent-instance-tests.el --- A session going on under another instance -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-claude)
  (require 'romacs-agent-codex))

(defconst romacs-agent-instance-tests--id "5e55105e-0000-4000-8000-000000000000")

(defmacro romacs-agent-instance-tests--with (&rest body)
  "BODY with instances claude (directory `one'), claude2 (`two') and
codex, and `session', an ended Claude Code session under claude whose
conversation is in `one'; `told' collects what a task is told."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-switch-" t))
          (one (expand-file-name "one" root)) (two (expand-file-name "two" root))
          (codex-home (expand-file-name "codex" root))
          (romacs-agent-providers (list (cons "claude" one) (cons "claude2" two) (cons "codex" codex-home)))
          (romacs-agent--sessions (make-hash-table :test #'equal))
          (romacs-agent--children (make-hash-table :test #'equal))
          (romacs-agent--room (make-hash-table :test #'equal))
          (romacs-agent-tools--answered (make-hash-table :test #'equal))
          (romacs-agent-tools--moving (make-hash-table :test #'equal))
          (romacs-agent-tools--pending (make-hash-table :test #'equal))
          (session (romacs-agent--make :id romacs-agent-instance-tests--id :harness 'claude :provider "claude"
                                       :config-dir one :cwd root :title "lane" :state 'ended :turns 2 :caught-up t
                                       :dir (expand-file-name "session" root)
                                       :options '(:model "opus" :parent "lead" :wake "end")))
          (told nil))
     (dolist (dir (list one two codex-home (romacs-agent-dir session))) (make-directory dir t))
     (cl-flet ((put (file text) (let ((path (expand-file-name file one)))
                                  (make-directory (file-name-directory path) t)
                                  (write-region text nil path))))
       (put (format "projects/-proj/%s.jsonl" romacs-agent-instance-tests--id) "{\"turn\":1}\n{\"turn\":2}\n")
       (put (format "projects/-proj/%s/tool-results/r1.txt" romacs-agent-instance-tests--id) "result")
       (put (format "file-history/%s/f1" romacs-agent-instance-tests--id) "before")
       (put (format "session-env/%s/env" romacs-agent-instance-tests--id) "X=1"))
     (puthash (romacs-agent-id session) session romacs-agent--sessions)
     (romacs-agent--manifest session nil)
     (cl-letf (((symbol-function 'romacs-agent-tools--tell) (lambda (task text) (push (cons task text) told) task))
               ((symbol-function 'run-at-time) #'ignore))
       (unwind-protect (progn ,@body) (delete-directory root t)))))

(ert-deftest romacs-agent-instance-moves-the-conversation ()
  (romacs-agent-instance-tests--with
    ;; Not running: in effect at once.
    (should (romacs-agent-select session "opus" nil "claude2"))
    (should (equal (romacs-agent-provider session) "claude2"))
    (should (equal (romacs-agent-config-dir session) two))
    (dolist (file (list (format "projects/-proj/%s.jsonl" romacs-agent-instance-tests--id)
                        (format "projects/-proj/%s/tool-results/r1.txt" romacs-agent-instance-tests--id)
                        (format "file-history/%s/f1" romacs-agent-instance-tests--id)
                        (format "session-env/%s/env" romacs-agent-instance-tests--id)))
      (should (file-exists-p (expand-file-name file two))))
    ;; The same session: its id, tasks and options; its record says where it runs.
    (should (equal (plist-get (romacs-agent-options session) :parent) "lead"))
    (let ((manifest (romacs-agent--read-json (expand-file-name "manifest.json" (romacs-agent-dir session)))))
      (should (equal (gethash "provider" manifest) "claude2"))
      (should (equal (gethash "configDir" manifest) two)))
    ;; And back: what it said there comes with it.
    (write-region "{\"turn\":3}\n" nil (expand-file-name (format "projects/-proj/%s.jsonl" romacs-agent-instance-tests--id) two) t)
    (romacs-agent-select session "opus" nil "claude")
    (should (equal (romacs-agent-provider session) "claude"))
    (should (string-match-p "turn\":3" (with-temp-buffer
                                        (insert-file-contents (expand-file-name (format "projects/-proj/%s.jsonl" romacs-agent-instance-tests--id) one))
                                        (buffer-string))))))

(ert-deftest romacs-agent-instance-a-restart-under-way-takes-the-last-choice ()
  (romacs-agent-instance-tests--with
    (setf (romacs-agent-pending-selection session) (list :state 'restarting :model "opus" :provider "claude2"))
    (romacs-agent-select session "opus" nil "claude")
    (should (equal (plist-get (romacs-agent-pending-selection session) :provider) "claude"))))

(ert-deftest romacs-agent-instance-a-fork-moves-once-it-has-spoken ()
  (romacs-agent-instance-tests--with
    (setf (romacs-agent-options session) (append '(:fork "5ource00-0000-4000-8000-000000000000") (romacs-agent-options session)))
    (should-error (romacs-agent-select session "opus" nil "claude2") :type 'user-error)
    (setf (romacs-agent-input-seq session) 3)
    (should (romacs-agent-select session "opus" nil "claude2"))))

(ert-deftest romacs-agent-instance-a-moved-import-is-not-its-old-conversation ()
  (romacs-agent-instance-tests--with
    (let* ((thread "7h7ead00-0000-4000-8000-000000000000")
           (process-environment (cons (concat "XDG_STATE_HOME=" root "/state") process-environment))
           (id (romacs-agent--import-id 'claude one thread))
           (dir (expand-file-name id (romacs-agent--state-root))))
      (make-directory dir t)
      ;; Recorded under its identity on claude, since moved to claude2.
      (romacs-agent--write-json (expand-file-name "manifest.json" dir)
                                (romacs-mcp--object "id" id "harness" "claude" "provider" "claude2" "configDir" two
                                                    "harnessSession" thread "cwd" root "title" "imported"
                                                    "hostSocket" (expand-file-name "none.sock" root)))
      (should-not (romacs-agent--conversation 'claude one thread))
      (should (eq (romacs-agent--conversation 'claude two thread) (romacs-agent-get id))))))

(ert-deftest romacs-agent-instance-refusals-and-a-failed-move ()
  (romacs-agent-instance-tests--with
    ;; Another harness is a handoff, not a move; an unknown instance, nothing.
    (should-error (romacs-agent-select session "opus" nil "codex") :type 'user-error)
    (should-error (romacs-agent-select session "opus" nil "nosuch") :type 'user-error)
    ;; A copy that fails leaves it where it was, and says so.
    (let ((errors nil))
      (cl-letf (((symbol-function 'copy-file) (lambda (&rest _) (error "Disk full")))
                ((symbol-function 'copy-directory) (lambda (&rest _) (error "Disk full"))))
        (let ((romacs-agent-event-functions (list (lambda (_s e) (when (eq (plist-get e :type) 'error) (push (plist-get e :text) errors))))))
          (romacs-agent-select session "opus" nil "claude2")))
      (should (equal (romacs-agent-provider session) "claude"))
      (should (equal (romacs-agent-config-dir session) one))
      (should (string-match-p "Not moved to claude2: Disk full" (car errors))))))

(ert-deftest romacs-agent-instance-a-usage-limit ()
  (romacs-agent-instance-tests--with
    ;; Claude Code's own words: its usage after a request, and a turn refused.
    (let ((limit (romacs-agent--claude-events
                  session (romacs-mcp--object "type" "rate_limit_event"
                                              "rate_limit_info" (romacs-mcp--object "status" "rejected" "rateLimitType" "seven_day"
                                                                                    "resetsAt" 4102444800 "utilization" 1))))
          (refused (romacs-agent--claude-events
                    session (romacs-mcp--object "type" "result" "is_error" t "api_error" "usage_limit_reached"
                                                "result" "You've hit your weekly limit"))))
      (should (eq (plist-get (car limit) :status) 'rejected))
      (should (plist-get (car refused) :limit))
      (romacs-agent--raise session (append (car limit) (list :time 100.0)))
      (romacs-agent--raise session (append (car refused) (list :seq 9))))
    ;; An older report replayed does not undo it.
    (romacs-agent--raise session (list :type 'rate-limit :status 'allowed :time 50.0))
    (should-not (romacs-agent-room "claude"))
    (should (romacs-agent-room "claude2"))
    ;; Where it could go on: its harness's instances first.
    (should (equal (mapcar (lambda (a) (plist-get a :provider)) (romacs-agent-alternatives session)) '("claude2" "codex")))
    (let ((record (romacs-agent-tools--task session)))
      (should (equal (gethash "provider" (gethash "limit" record)) "claude"))
      (should (eq (gethash "lossless" (aref (gethash "alternatives" (gethash "limit" record)) 0)) t)))
    (let ((wake (romacs-agent-tools--wake-text session (list 9 (romacs-agent-reply session) t))))
      (should (string-match-p "Usage limit on claude" wake))
      (should (string-match-p "claude2 (the same conversation moves), codex (a handoff on a brief)" wake))
      ;; Still one wake, read back whole.
      (should (romacs-agent-tools--wake-parse wake)))))

(ert-deftest romacs-agent-instance-codex-limits ()
  (romacs-agent-instance-tests--with
    (let ((codex (romacs-agent--make :id "c" :harness 'codex :provider "codex")))
      ;; The fuller window binds: a spent weekly one under a light primary.
      (let ((event (car (romacs-agent--codex-notified
                         codex "account/rateLimits/updated"
                         (romacs-mcp--object "rateLimits"
                                             (romacs-mcp--object "primary" (romacs-mcp--object "usedPercent" 20 "resetsAt" 100)
                                                                 "secondary" (romacs-mcp--object "usedPercent" 100 "resetsAt" 200
                                                                                                 "windowDurationMins" 10080)))))))
        (should (eq (plist-get event :status) 'rejected))
        (should (= (plist-get event :resets) 200)))
      ;; Both spent: no room until the later of them resets.
      (let ((event (car (romacs-agent--codex-notified
                         codex "account/rateLimits/updated"
                         (romacs-mcp--object "rateLimits"
                                             (romacs-mcp--object "primary" (romacs-mcp--object "usedPercent" 100 "resetsAt" 1000)
                                                                 "secondary" (romacs-mcp--object "usedPercent" 100 "resetsAt" 3000)))))))
        (should (= (plist-get event :resets) 3000)))
      ;; A turn refused at the limit, in Codex's own words.
      (should (plist-get (romacs-agent--codex-done
                          codex (romacs-mcp--object "status" "failed"
                                                    "error" (romacs-mcp--object "message" "You've hit your usage limit"
                                                                                "codexErrorInfo" "usageLimitExceeded")))
                         :limit)))))

(ert-deftest romacs-agent-instance-on-limit-moves-a-lane ()
  (romacs-agent-instance-tests--with
    (let ((romacs-agent-switch-on-limit 'lossless) (sent nil))
      (cl-letf (((symbol-function 'romacs-agent--enqueue) (lambda (_s item) (push (car item) sent))))
        ;; Replayed, not live: nothing moves.
        (setf (romacs-agent-caught-up session) nil)
        (romacs-agent--raise session (list :type 'done :error t :limit t :text "old limit" :seq 5))
        (should (equal (romacs-agent-provider session) "claude"))
        (setf (romacs-agent-caught-up session) t)
        (romacs-agent--raise session (list :type 'rate-limit :status 'rejected :time 1.0))
        (romacs-agent--raise session (list :type 'done :error t :limit t :text "You've hit your weekly limit" :seq 9))
        ;; Moved, told to go on, and its parent not woken for it.
        (should (equal (romacs-agent-provider session) "claude2"))
        (should (string-match-p "stopped at claude's usage limit; you now run on claude2" (car sent)))
        (should-not (romacs-agent-tools--wanted-p session 9))
        ;; One that cannot move wakes its parent with its end, as without the option.
        (let ((parent (romacs-agent--make :id "lead" :harness 'claude :state 'running)))
          (puthash "lead" parent romacs-agent--sessions)
          (setq sent nil)
          (cl-letf (((symbol-function 'copy-file) (lambda (&rest _) (error "Disk full")))
                    ((symbol-function 'copy-directory) (lambda (&rest _) (error "Disk full"))))
            ;; Now claude2 is spent, and claude has room again.
            (romacs-agent--raise session (list :type 'rate-limit :status 'rejected :time 2.0))
            (puthash "claude" (list :status 'allowed :time 3.0) romacs-agent--room)
            (romacs-agent--raise session (list :type 'done :error t :limit t :text "limit again" :seq 12)))
          (should (equal (romacs-agent-provider session) "claude2"))
          (should-not sent)
          (should (romacs-agent-tools--wanted-p session 12))
          (should (gethash "lead" romacs-agent-tools--pending)))))))

(ert-deftest romacs-agent-instance-the-tool ()
  (romacs-agent-instance-tests--with
    (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () "lead")))
      (setf (romacs-agent-state session) 'running)
      ;; A running task: armed, in effect once its turn ends.
      (let ((record (romacs-agent-tools--switch (romacs-mcp--object "taskId" romacs-agent-instance-tests--id
                                                                    "provider" "claude2" "next" "go on"))))
        (should (equal (plist-get (romacs-agent-pending-selection session) :provider) "claude2"))
        (should (string-match-p "moves once its running turn ends" (gethash "next" record)))
        (should (equal (cdr (car told)) "go on")))
      ;; Another session's task: refused.
      (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () "other")))
        (should-error (romacs-agent-tools--switch (romacs-mcp--object "taskId" romacs-agent-instance-tests--id "provider" "claude2"))
                      :type 'user-error)))))

(provide 'romacs-agent-instance-tests)
;;; romacs-agent-instance-tests.el ends here
