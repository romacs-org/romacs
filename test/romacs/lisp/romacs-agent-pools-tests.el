;;; romacs-agent-pools-tests.el --- A harness's accounts as one pool -*- lexical-binding: t; -*-
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
  (require 'romacs-agent-claude)
  (require 'romacs-agent-codex)
  (require 'romacs-agent-ui))

(defmacro romacs-agent-pools-tests--with (&rest body)
  "BODY with three Claude accounts and one Codex account, a private state
root and no limit known; `spend' marks an account's limit reached until
a time, `allow' marks it allowed."
  (declare (indent 0))
  `(let* ((root (make-temp-file "romacs-pools-" t))
          (process-environment (cons (concat "XDG_STATE_HOME=" root "/state") process-environment))
          (romacs-agent-providers (mapcar (lambda (name) (let ((dir (expand-file-name name root)))
                                                           (make-directory dir t)
                                                           (cons name dir)))
                                          '("claude" "claude2" "claude3" "codex")))
          (romacs-agent-pools nil)
          (romacs-agent--room (make-hash-table :test #'equal))
          (romacs-agent--room-loaded t)
          (romacs-agent--sessions (make-hash-table :test #'equal)))
     (cl-letf (((symbol-function 'romacs-agent--check-companions) #'ignore))
       (cl-flet ((spend (name resets &optional time)
                   (romacs-agent--note-room (romacs-agent--make :provider name)
                                            (list :status 'rejected :resets resets :window "five_hour" :time (or time 1.0))))
                 (allow (name &optional time)
                   (romacs-agent--note-room (romacs-agent--make :provider name)
                                            (list :status 'allowed :used 0.2 :window "five_hour" :time (or time 2.0))))
                 (names (instances) (mapcar (lambda (i) (plist-get i :name)) instances)))
         (unwind-protect (progn ,@body) (delete-directory root t))))))

(ert-deftest romacs-agent-pools-order-and-pick ()
  (romacs-agent-pools-tests--with
    (should (equal (names (romacs-agent-pool 'claude)) '("claude" "claude2" "claude3")))
    (should (equal (names (romacs-agent-pool 'codex)) '("codex")))
    ;; Its own order, when given.
    (let ((romacs-agent-pools '((claude "claude3" "claude"))))
      (should (equal (names (romacs-agent-pool 'claude)) '("claude3" "claude"))))
    (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude"))
    ;; Spent until later: the next with room.
    (spend "claude" (+ (float-time) 3600))
    (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude2"))
    (spend "claude2" (+ (float-time) 600))
    (spend "claude3" (+ (float-time) 7200))
    ;; None with room: the one back first.
    (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude2"))
    ;; Its limit reset: new work draws on the first again.
    (spend "claude" (- (float-time) 1) 3.0)
    (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude"))))

(ert-deftest romacs-agent-pools-a-harness-names-its-pool-an-instance-pins ()
  (romacs-agent-pools-tests--with
    (spend "claude" (+ (float-time) 3600))
    ;; The harness, a symbol: its pool.
    (should (equal (plist-get (romacs-agent--start-instance 'claude) :name) "claude2"))
    (should (equal (plist-get (romacs-agent--start-instance 'codex) :name) "codex"))
    ;; An instance's name pins it, even one named as its harness.
    (should (equal (plist-get (romacs-agent--start-instance "claude") :name) "claude"))
    (should (equal (plist-get (romacs-agent--start-instance "claude3") :name) "claude3"))
    ;; Nothing named: the first enabled instance's harness's pool.
    (should (equal (plist-get (romacs-agent--start-instance nil) :name) "claude2"))
    ;; An agent's tool call names a harness by its name.
    (should (eq (romacs-agent-pool-name "claude") 'claude))
    (should (equal (romacs-agent-pool-name "claude2") "claude2"))))

(ert-deftest romacs-agent-pools-pickers-name-harnesses ()
  (romacs-agent-pools-tests--with
    (cl-letf (((symbol-function 'romacs-agent--models) (lambda (_) '(("opus" :label "Opus")))))
      (let ((rows (romacs-agent--picker-rows)))
        (should (equal (delete-dups (mapcar (lambda (row) (plist-get (cdr row) :provider)) rows)) '(claude codex)))
        (should-not (cl-find-if (lambda (row) (string-match-p "claude2\\|Claude2" (car row))) rows)))
      ;; Accounts only when asked for.
      (should (member "claude2" (mapcar (lambda (row) (plist-get (cdr row) :provider)) (romacs-agent--picker-rows t))))
      (let ((romacs-agent-pick-accounts t))
        (should (member "claude3" (mapcar (lambda (row) (plist-get (cdr row) :provider)) (romacs-agent--picker-rows))))))))

(ert-deftest romacs-agent-pools-limits-are-kept-across-a-restart ()
  (romacs-agent-pools-tests--with
    (let ((writes 0))
      (cl-letf* ((keep (symbol-function 'romacs-agent--keep-room))
                 ((symbol-function 'romacs-agent--keep-room) (lambda () (cl-incf writes) (funcall keep))))
        (spend "claude" 5000.0 1.0)
        (allow "claude2" 2.0)
        ;; Each request's use alone is not written.
        (romacs-agent--note-room (romacs-agent--make :provider "claude2")
                                 (list :status 'allowed :used 0.4 :window "five_hour" :time 3.0))
        (should (= writes 2))))
    ;; A new server reads what was kept.
    (let ((romacs-agent--room (make-hash-table :test #'equal)) (romacs-agent--room-loaded nil))
      (should (eq (plist-get (romacs-agent-room-of "claude") :status) 'rejected))
      (should (= (plist-get (romacs-agent-room-of "claude") :resets) 5000.0)))))

(ert-deftest romacs-agent-pools-move-on-to-the-next-account ()
  (romacs-agent-pools-tests--with
    (let ((session (romacs-agent--make :id "s" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                       :dir (make-temp-file "s" t) :limited '(:provider "claude")))
          (selected nil) (queued nil) (moved nil))
      (puthash "s" session romacs-agent--sessions)
      (spend "claude" (+ (float-time) 3600))
      (cl-letf (((symbol-function 'romacs-agent-select)
                 (lambda (_session _model _effort provider) (setq selected provider) t))
                ((symbol-function 'romacs-agent--enqueue) (lambda (_s item) (push (car item) queued))))
        (let ((romacs-agent-moved-functions (list (lambda (_s from to) (push (cons from to) moved)))))
          (should (equal (romacs-agent-move-on session) "claude2"))
          (should (equal selected "claude2"))
          ;; Told to go on only once the move took effect.
          (should-not queued)
          (romacs-agent--raise session '(:type selection :from (:provider "claude") :to (:provider "claude2")))
          (should (string-match-p "claude's usage limit; you now run on claude2" (car queued)))
          (should (equal moved '(("claude" . "claude2"))))
          ;; A move that did not take effect: nothing to go on with.
          (setq queued nil)
          (romacs-agent-move-on session)
          (romacs-agent--raise session '(:type selection :from (:provider "claude") :to (:provider "claude")))
          (should-not queued)
          (should (equal (car moved) '("claude" . "claude"))))
        (spend "claude2" (+ (float-time) 3600))
        (spend "claude3" (+ (float-time) 3600))
        (should-error (romacs-agent-move-on session) :type 'user-error)))))

(ert-deftest romacs-agent-pools-shown-to-agents-and-the-user ()
  (romacs-agent-pools-tests--with
    (should-not (romacs-agent-ui--pools-line))
    (spend "claude" (+ (float-time) 3600))
    (allow "claude2")
    (let ((line (romacs-agent-ui--pools-line)))
      (should (equal (substring-no-properties line) "⛽claude 2/3"))
      (should (string-match-p "\\`claude: rejected, resets " (get-text-property 0 'help-echo line))))
    (let* ((pools (gethash "pools" (romacs-agent-tools--capabilities (romacs-mcp--object))))
           (claude (cl-find "claude" pools :key (lambda (p) (gethash "harness" p)) :test #'equal)))
      (should (equal (gethash "next" claude) "claude2"))
      (should (equal (mapcar (lambda (a) (list (gethash "providerInstanceId" a) (gethash "room" a)))
                             (gethash "accounts" claude))
                     '(("claude" :false) ("claude2" t) ("claude3" t)))))))

(ert-deftest romacs-agent-pools-resume-at-the-reset ()
  ;; Every account spent, the first back in a few seconds: it resumes by
  ;; itself then, journaled, kept across a restart, on the first with room.
  (romacs-agent-pools-tests--with
    (let* ((dir (make-temp-file "s" t))
           (session (romacs-agent--make :id "r" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                        :dir dir :done-seq 4 :input-seq 3 :limited '(:provider "claude")))
           (soon (+ (float-time) 5)) (armed nil) (told nil) (moved nil)
           (romacs-agent-tools--resumes (make-hash-table :test #'eq :weakness 'key))
           (romacs-agent-tools--moving (make-hash-table :test #'equal))
           (romacs-agent-switch-delay 0) (romacs-agent-resume-margin 1))
      (puthash "r" session romacs-agent--sessions)
      (romacs-agent--manifest session nil)
      (spend "claude" soon) (spend "claude2" (+ soon 600)) (spend "claude3" (+ soon 900))
      (cl-letf (((symbol-function 'run-at-time) (lambda (delay _ function &rest args) (push (list delay function args) armed) 'timer))
                ((symbol-function 'cancel-timer) #'ignore)
                ((symbol-function 'romacs-agent-tools--wake) #'ignore)
                ((symbol-function 'romacs-agent-tools--tell) (lambda (_s text) (push text told)))
                ((symbol-function 'romacs-agent-move-on) (lambda (_s why) (push why moved))))
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        ;; Scheduled at the earliest reset plus its margin, said and kept.
        (should (= (romacs-agent-resume-time session) (+ soon 1)))
        (should (cl-find #'romacs-agent-tools--resume armed :key #'cadr))
        (let ((journal (with-temp-buffer (insert-file-contents (expand-file-name "journal.jsonl" dir)) (buffer-string))))
          (should (string-match-p "\"type\":\"resume\",\"state\":\"scheduled\"" journal)))
        ;; Kept in its manifest: a restarted server, loading it, arms it again.
        (let ((manifest (romacs-agent--read-json (expand-file-name "manifest.json" dir))))
          (should (= (gethash "resumeAt" manifest) (+ soon 1)))
          (clrhash romacs-agent-tools--resumes)
          (run-hook-with-args 'romacs-agent-loaded-functions session manifest))
        (should (= (romacs-agent-resume-time session) (+ soon 1)))
        ;; Due, its own account back: told to go on there.
        (spend "claude" (- (float-time) 1) 9.0)
        (romacs-agent-tools--resume session (+ soon 1))
        (should (equal told (list romacs-agent-tools--resumed)))
        (should-not (romacs-agent-resume-time session))
        (should (string-match-p "\"state\":\"fired\",\"on\":\"claude\""
                                (with-temp-buffer (insert-file-contents (expand-file-name "journal.jsonl" dir)) (buffer-string))))
        ;; Another account back first: it moves there.
        (spend "claude" (+ (float-time) 600) 10.0)
        (allow "claude2" 11.0)
        (romacs-agent-tools--arm-resume session 42.0)
        (romacs-agent-tools--resume session 42.0)
        (should (equal moved '("limit reset")))
        ;; A message cancels a waiting resume, journaled.
        (romacs-agent-tools--arm-resume session 43.0)
        (romacs-agent-tools--limit-on-event session '(:type input))
        (should-not (romacs-agent-resume-time session))
        ;; Its manifest says so at once: nothing written later brings it back.
        (should-not (gethash "resumeAt" (romacs-agent--read-json (expand-file-name "manifest.json" dir))))
        ;; A reset already used schedules nothing again.
        (spend "claude" soon 13.0) (spend "claude2" (+ soon 600) 13.0)
        (puthash session soon romacs-agent-tools--resets-used)
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        (should-not (romacs-agent-resume-time session))
        ;; Turned off: nothing scheduled, none fires, none kept is armed again.
        (let ((romacs-agent-resume-at-reset nil))
          (spend "claude" (+ soon 1000) 14.0)
          (romacs-agent-tools--switch-on-limit session '(:limit t))
          (should-not (romacs-agent-resume-time session))
          (setq told nil moved nil)
          (romacs-agent-tools--arm-resume session 44.0)
          (romacs-agent-tools--resume session 44.0)
          (should-not (or told moved))
          (romacs-agent-tools--keep-resume session 45.0)
          (run-hook-with-args 'romacs-agent-loaded-functions session
                              (romacs-agent--read-json (expand-file-name "manifest.json" dir)))
          (should-not (romacs-agent-resume-time session))
          (should-not (gethash "resumeAt" (romacs-agent--read-json (expand-file-name "manifest.json" dir)))))))))

(ert-deftest romacs-agent-pools-a-wait-is-its-limits-alone ()
  (romacs-agent-pools-tests--with
    (let* ((dir (make-temp-file "w" t))
           (session (romacs-agent--make :id "w" :harness 'claude :provider "claude" :state 'idle :caught-up t
                                        :dir dir :done-seq 4 :input-seq 3 :limited '(:provider "claude")))
           (armed nil) (moves 0) (selects 0)
           (romacs-agent-tools--moving (make-hash-table :test #'equal))
           (romacs-agent-tools--resumes (make-hash-table :test #'eq :weakness 'key)))
      (puthash "w" session romacs-agent--sessions)
      (spend "claude" (+ (float-time) 600))
      (cl-letf (((symbol-function 'run-at-time) (lambda (_d _r function &rest args) (push (cons function args) armed) 'timer))
                ((symbol-function 'romacs-agent-tools--wake) #'ignore)
                ((symbol-function 'romacs-agent-select) (lambda (&rest _) (cl-incf selects) (= selects 1))))
        ;; A message after the limit: the wait it armed moves nothing.
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        (let ((wait (cl-find #'romacs-agent-tools--hand-over armed :key #'car)))
          (romacs-agent-tools--limit-on-event session '(:type input))
          (cl-letf (((symbol-function 'romacs-agent-move-on) (lambda (&rest _) (cl-incf moves))))
            (apply (car wait) (cdr wait))))
        (should (= moves 0))
        ;; The same move asked again while it is pending keeps its bookkeeping.
        (romacs-agent-move-on session)
        (setf (romacs-agent-pending-selection session) '(:state armed :provider "claude2"))
        (cl-letf (((symbol-function 'romacs-agent--wanted) (lambda (_) '(:provider "claude2"))))
          (romacs-agent-move-on session))
        (should (gethash session romacs-agent--moving-on))
        (setf (romacs-agent-pending-selection session) nil)
        ;; The rest spent during the wait: said in the journal, with the reset.
        (remhash session romacs-agent--moving-on)
        (setq armed nil)
        (romacs-agent-tools--switch-on-limit session '(:limit t))
        (spend "claude2" (+ (float-time) 600) 5.0) (spend "claude3" (+ (float-time) 900) 5.0)
        (let ((wait (cl-find #'romacs-agent-tools--hand-over armed :key #'car)))
          (apply (car wait) (cdr wait)))
        (should (string-match-p "\"to\":null,\"why\":\"every claude account is at its limit\",\"resets\":[0-9]"
                                (with-temp-buffer (insert-file-contents (expand-file-name "journal.jsonl" dir)) (buffer-string))))))))

(ert-deftest romacs-agent-pools-the-room-file-has-one-writer ()
  (romacs-agent-pools-tests--with
    (let* ((file (romacs-agent--room-file)) (lock (concat file ".lock")) (seen nil))
      (romacs-agent--with-lock file (lambda () (setq seen (file-directory-p lock))))
      (should seen)
      (should-not (file-exists-p lock))
      ;; Held by another: not run, tried again shortly.
      (make-directory lock t)
      (let ((retried nil))
        (cl-letf (((symbol-function 'run-at-time) (lambda (&rest args) (setq retried args))))
          (romacs-agent--with-lock file (lambda () (setq seen 'unlocked))))
        (should-not (eq seen 'unlocked))
        (should (eq (nth 2 retried) #'romacs-agent--with-lock)))
      ;; A crashed writer's lock is taken over.
      (set-file-times lock (time-subtract (current-time) 60))
      (romacs-agent--with-lock file (lambda () (setq seen 'ran)))
      (should (eq seen 'ran))
      (should-not (file-exists-p lock)))))

(ert-deftest romacs-agent-pools-switching-to-a-harness-is-its-pool ()
  (romacs-agent-pools-tests--with
    (let ((session (romacs-agent--make :id "x" :harness 'claude :provider "claude"))
          (to nil))
      (puthash "x" session romacs-agent--sessions)
      (cl-letf (((symbol-function 'romacs-agent-tools--caller) (lambda () "x"))
                ((symbol-function 'romacs-agent-tools--handed-off) (lambda (_task provider &rest _) (setq to provider))))
        (romacs-agent-tools--switch (romacs-mcp--object "provider" "codex"))
        (should (eq to 'codex))))))

;;; romacs-agent-pools-tests.el ends here
