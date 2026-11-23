;;; romacs-agent-pool-auth-tests.el --- Failover after expired account credentials -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

(require 'ert)
(require 'cl-lib)

(eval-and-compile
  (when-let* ((file (or load-file-name (bound-and-true-p byte-compile-current-file)))
              (load-path (cons (file-name-directory file) load-path)))
    (unless (fboundp 'romacs-agent-pools-tests--with)
      ;; Reuse the fixture without running the pool suite again in this file.
      (cl-letf (((symbol-function 'ert-set-test) #'ignore))
        (load "romacs-agent-pools-tests" nil t)))
    (require 'romacs-agent-test-attached)))

(defconst romacs-agent-pool-auth-tests--reason
  "Failed to authenticate: OAuth session expired and could not be refreshed")

(defun romacs-agent-pool-auth-tests--call (body)
  "Call BODY with the private account ROOT, credential STAMPS and SPEND."
  (romacs-agent-pools-tests--with
     (let ((stamps (make-hash-table :test #'equal))
           (romacs-agent--children (make-hash-table :test #'equal))
           (romacs-agent--attempts (make-hash-table :test #'eq))
           (romacs-agent--held-notes (make-hash-table :test #'equal)))
       ;; Where a launch reads it: the directory the harness gets.
       (cl-letf (((symbol-function 'romacs-agent--home-stamp)
                  (lambda (_harness dir)
                    (romacs-agent-pool-auth-tests--stamp
                     (gethash (romacs-agent-pool-auth-tests--instance-of dir) stamps "before-login"))))
                 ((symbol-function 'romacs-agent--keep-soon) #'ignore))
         (should (equal (names (romacs-agent-pool 'claude)) '("claude" "claude2" "claude3")))
         (allow "claude2" (float-time))
         (funcall body root stamps #'spend)))))

(defun romacs-agent-pool-auth-tests--stamp (label)
  "A credentials stamp named LABEL: a login that is another as LABEL is."
  (cons label (cons 0 1000000000)))

(defun romacs-agent-pool-auth-tests--instance-of (dir)
  "The name of the instance whose configuration directory DIR is."
  (car (cl-find-if (lambda (provider)
                     (and (stringp (cdr provider)) dir
                          (equal (file-name-as-directory (expand-file-name (cdr provider)))
                                 (file-name-as-directory (expand-file-name dir)))))
                   romacs-agent-providers)))

(defun romacs-agent-pool-auth-tests--run-note (session)
  "Journal SESSION's run note as its supervisor would, its launch's stamp in it."
  (romacs-agent-pool-auth-tests--record session "note" (json-parse-string (romacs-agent--run-note session))))

(defmacro romacs-agent-pool-auth-tests--with (&rest body)
  "Run BODY with the pool fixture and an opaque credential stamp per instance."
  (declare (indent 0))
  `(romacs-agent-pool-auth-tests--call (lambda (root _stamps _spend) ,@body)))

(defun romacs-agent-pool-auth-tests--record (session kind value)
  "Feed SESSION its next supervisor record of KIND carrying VALUE."
  (romacs-agent--record session (romacs-mcp--object
                                "seq" (1+ (romacs-agent-seq session)) "t" (float-time)
                                kind value)))

(defun romacs-agent-pool-auth-tests--fail (session &optional other-error)
  "Feed SESSION the assistant and result pair seen on OAuth expiry.
OTHER-ERROR replaces the structured code while keeping the error wording."
  (let ((reason romacs-agent-pool-auth-tests--reason))
    (romacs-agent-pool-auth-tests--record
     session "out"
     (romacs-mcp--object "type" "assistant" "error" (or other-error "authentication_failed")
                         "is_api_error_message" t
                         "message" (romacs-mcp--object
                                    "id" "synthetic-error" "model" "<synthetic>"
                                    "content" (vector (romacs-mcp--object "type" "text" "text" reason)))))
    (romacs-agent-pool-auth-tests--record
     session "out"
     (romacs-mcp--object "type" "result" "subtype" "success" "is_error" t
                         "terminal_reason" "api_error" "result" reason))))

(defun romacs-agent-pool-auth-tests--init (session)
  "Feed SESSION the Claude launch's init record."
  (romacs-agent-pool-auth-tests--record
   session "out" (romacs-mcp--object "type" "system" "subtype" "init"
                                    "session_id" (romacs-agent-id session) "model" "opus")))

(defun romacs-agent-pool-auth-tests--session (root provider)
  "Create a catch-up-complete Claude session for PROVIDER under ROOT."
  (let ((session (romacs-agent--create :id (romacs-agent--uuid) :harness 'claude
                                      :provider provider :state 'starting :caught-up t
                                      :cwd root :config-dir (cdr (assoc provider romacs-agent-providers)))))
    (romacs-agent--manifest session nil)
    (romacs-agent-pool-auth-tests--run-note session)
    session))

(defun romacs-agent-pool-auth-tests--unavailable (provider)
  "Assert that PROVIDER's room carries the authentication failure."
  (let ((room (romacs-agent-room-of provider)))
    (should (eq (plist-get room :status) 'unavailable))
    (should (equal (plist-get room :reason) romacs-agent-pool-auth-tests--reason))
    (should-not (plist-get room :resets))))

(ert-deftest romacs-agent-pool-auth-marks-only-authentication-failures ()
  (romacs-agent-pool-auth-tests--with
    (let ((ordinary (romacs-agent-pool-auth-tests--session root "claude2"))
          (expired (romacs-agent-pool-auth-tests--session root "claude")))
      (romacs-agent-pool-auth-tests--fail ordinary "overloaded")
      (should (romacs-agent-failed ordinary))
      (should (eq (plist-get (romacs-agent-room-of "claude2") :status) 'allowed))
      (romacs-agent-pool-auth-tests--fail expired)
      (romacs-agent-pool-auth-tests--unavailable "claude"))))

(ert-deftest romacs-agent-pool-auth-routing-skips-unavailable ()
  (romacs-agent-pool-auth-tests--with
    (let ((first (romacs-agent-pool-auth-tests--session root "claude"))
          (second (romacs-agent-pool-auth-tests--session root "claude2")))
      (romacs-agent-pool-auth-tests--fail first)
      (should-not (romacs-agent-room "claude"))
      (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude2"))
      (should (equal (mapcar (lambda (a) (plist-get a :provider)) (romacs-agent-alternatives second))
                     '("claude3" "codex")))
      (funcall _spend "claude2" (+ (float-time) 600) (float-time))
      (romacs-agent-pool-auth-tests--fail (romacs-agent-pool-auth-tests--session root "claude3"))
      (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude2"))
      (romacs-agent-pool-auth-tests--fail second)
      (should-not (romacs-agent-pool-pick 'claude)))))

(ert-deftest romacs-agent-pool-auth-capabilities-explain-unavailability ()
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-tests--fail (romacs-agent-pool-auth-tests--session root "claude"))
    (cl-letf (((symbol-function 'romacs-agent--want-models) #'ignore))
      (let* ((answer (romacs-agent-tools--capabilities (romacs-mcp--object)))
             (pool (cl-find "claude" (gethash "pools" answer)
                            :key (lambda (p) (gethash "harness" p)) :test #'equal))
             (account (cl-find "claude" (gethash "accounts" pool)
                               :key (lambda (a) (gethash "providerInstanceId" a)) :test #'equal)))
        (should (equal (gethash "status" account) "unavailable"))
        (should (eq (gethash "room" account) :false))
        (should (equal (gethash "reason" account) romacs-agent-pool-auth-tests--reason))
        (should (eq (gethash "resets" account) :null))
        (should (equal (gethash "next" pool) "claude2"))))))

(defun romacs-agent-pool-auth-tests--succeed (session)
  "Feed SESSION a successful Claude result."
  (romacs-agent-pool-auth-tests--record
   session "out" (romacs-mcp--object "type" "result" "subtype" "success"
                                    "is_error" :false "result" "Finished the work.")))

(ert-deftest romacs-agent-pool-auth-successful-turn-clears-the-mark ()
  (romacs-agent-pool-auth-tests--with
    (let ((expired (romacs-agent-pool-auth-tests--session root "claude"))
          (fresh (romacs-agent-pool-auth-tests--session root "claude"))
          (romacs-agent-room-functions nil))
      (romacs-agent-pool-auth-tests--fail expired)
      (romacs-agent-pool-auth-tests--unavailable "claude")
      (romacs-agent-pool-auth-tests--init fresh)
      (should (equal (romacs-agent-harness-session fresh) (romacs-agent-id fresh)))
      (romacs-agent-pool-auth-tests--unavailable "claude")
      (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude2"))
      (romacs-agent-note-room "claude"
                              '(:status allowed
                                :windows ((:name "five_hour" :status allowed :used 0.25)
                                          (:name "seven_day" :status allowed :used 0.50))))
      (romacs-agent-pool-auth-tests--unavailable "claude")
      (let ((windows (plist-get (romacs-agent-room-of "claude") :windows)))
        (should (equal (plist-get (cdr (assoc "five_hour" windows)) :used) 0.25))
        (should (equal (plist-get (cdr (assoc "seven_day" windows)) :used) 0.50))
        (romacs-agent-pool-auth-tests--succeed fresh)
        (should-not (romacs-agent-failed fresh))
        (should (romacs-agent-room "claude"))
        (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude"))
        (should-not (plist-get (romacs-agent-room-of "claude") :reason))
        (should (equal (plist-get (romacs-agent-room-of "claude") :windows) windows))))))

(ert-deftest romacs-agent-pool-auth-credentials-change-clears-the-mark ()
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-tests--fail (romacs-agent-pool-auth-tests--session root "claude"))
    (should-not (romacs-agent-room "claude"))
    (puthash "claude2" "other-login" _stamps)
    (should-not (romacs-agent-room "claude"))
    (puthash "claude" "after-login" _stamps)
    (should (romacs-agent-room "claude"))
    (should (equal (plist-get (romacs-agent-pool-pick 'claude) :name) "claude"))))

(ert-deftest romacs-agent-pool-auth-kept-across-restart ()
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-tests--fail (romacs-agent-pool-auth-tests--session root "claude"))
    (let ((romacs-agent--room (make-hash-table :test #'equal))
          (romacs-agent--room-loaded nil))
      (romacs-agent-pool-auth-tests--unavailable "claude")
      (should-not (romacs-agent-room "claude"))
      (puthash "claude" "after-restart-login" _stamps)
      (should (romacs-agent-room "claude")))))

(ert-deftest romacs-agent-pool-auth-stamp-follows-a-symlink ()
  (let ((read-stamp (symbol-function 'romacs-agent--credentials-stamp))
        (home-stamp (symbol-function 'romacs-agent--home-stamp)))
    (romacs-agent-pool-auth-tests--with
     (cl-letf (((symbol-function 'romacs-agent--home-stamp) home-stamp))
      (let ((target (expand-file-name ".credentials.json" (cdr (assoc "claude2" romacs-agent-providers))))
            (link (expand-file-name ".credentials.json" (cdr (assoc "claude" romacs-agent-providers)))))
        (write-region "{\"claudeAiOauth\":{\"accessToken\":\"before\"}}" nil target)
        (set-file-times target (time-subtract (current-time) 120))
        (make-symbolic-link target link)
        (let ((before (and read-stamp (funcall read-stamp "claude"))))
          (should before)
          (write-region "{\"claudeAiOauth\":{\"accessToken\":\"after\"}}" nil target)
          (set-file-times target (time-subtract (current-time) 60))
          (let ((after (funcall read-stamp "claude")))
            (should after)
            (should-not (equal after before)))))))))

(defun romacs-agent-pool-auth-tests--finish (session rejected)
  "Make the harness for SESSION reject credentials or complete its turn."
  (if (not rejected)
      (romacs-agent-pool-auth-tests--succeed session)
    (let ((pipe (romacs-agent-connection session)))
      (romacs-agent-pool-auth-tests--fail session)
      (setf (romacs-agent-connection session) nil)
      (delete-process pipe)
      (romacs-agent-pool-auth-tests--record session "exit" 1))))

(defun romacs-agent-pool-auth-tests--launch (launch session argv args)
  "Run LAUNCH for SESSION with ARGV and ARGS against a supervisor double."
  (cl-letf (((symbol-function 'romacs-agent--program) #'identity)
            ((symbol-function 'romacs-agent--route-here) #'ignore)
            ((symbol-function 'romacs-agent-events-own) #'ignore)
            ((symbol-function 'call-process)
             (lambda (program &rest _)
               (pcase program
                 ("systemd-run" 1)
                 ("setsid" 0)
                 (_ (ert-fail (list "Unexpected launch program" program)))))))
    (apply launch session argv args))
  (romacs-agent-pool-auth-tests--run-note session))

(defmacro romacs-agent-pool-auth-tests--transport (rejected &rest body)
  "Run BODY with a supervisor double rejecting the named accounts in REJECTED.
LAUNCHES records (SESSION PROVIDER ARGV), and WRITES records (PROVIDER PROMPT).
DRAIN runs the queued supervisor replies and one-shot callbacks in order."
  (declare (indent 1))
  `(let ((rejected ,rejected) (launches nil) (writes nil) (callbacks nil) (pipes nil)
         (launch (symbol-function 'romacs-agent--launch))
         (romacs-agent--launched (make-hash-table :test #'equal)))
     (cl-flet ((later (function &rest args)
                 (setq callbacks (nconc callbacks (list (cons function args))))
                 nil))
       (cl-letf (((symbol-function 'romacs-agent-events-then)
                  (lambda (_session callback) (funcall callback)))
                 ((symbol-function 'romacs-agent--launch)
                  (lambda (session argv &rest args)
                    (push (list session (romacs-agent-provider session) argv) launches)
                    (romacs-agent-pool-auth-tests--launch launch session argv args)))
                 ((symbol-function 'romacs-agent--attach)
                  (lambda (session &rest _)
                    (remhash (romacs-agent-id session) romacs-agent--launched)
                    (romacs-agent-test-attach session)
                    (push (romacs-agent-connection session) pipes)
                    (romacs-agent--apply-driver-state session '(:initialized t))
                    (later #'romacs-agent-pool-auth-tests--init session)
                    (romacs-agent-connection session)))
                 ((symbol-function 'romacs-agent--line)
                  (lambda (session wire &optional journal)
                    (should journal)
                    (let ((provider (romacs-agent-provider session)))
                      (push (list provider (gethash "content" (gethash "message" wire))) writes)
                      (later #'romacs-agent-pool-auth-tests--record session "in" journal)
                      (later #'romacs-agent-pool-auth-tests--finish session (member provider rejected)))))
                 ((symbol-function 'run-at-time)
                  (lambda (_delay repeat function &rest args)
                    (unless (or repeat (eq function #'romacs-agent--attempt-due))
                      (apply #'later function args)))))
         (cl-flet ((drain ()
                     (cl-loop repeat 80 while callbacks
                              do (let ((call (pop callbacks))) (apply (car call) (cdr call))))
                     (should-not callbacks)))
           (unwind-protect (progn ,@body)
             (dolist (pipe pipes) (when (process-live-p pipe) (delete-process pipe)))))))))

(ert-deftest romacs-agent-pool-auth-refusal-uses-the-attempts-stamp ()
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-tests--transport '("claude")
      (let ((read-stamp (symbol-function 'romacs-agent--home-stamp))
            (reads nil))
        (cl-letf (((symbol-function 'romacs-agent--home-stamp)
                   (lambda (harness dir)
                     (let ((stamp (funcall read-stamp harness dir)))
                       (push (list (romacs-agent-pool-auth-tests--instance-of dir) (car-safe stamp)) reads)
                       stamp))))
          (let* ((session (romacs-agent-start root "Read the fixture." "Pinned" "claude"))
                 (launch-reads reads))
            (should (equal (mapcar #'cadr launches) '("claude")))
            (puthash "claude" "after-login" _stamps)
            (drain)
            (should (romacs-agent-failed session))
            (should (romacs-agent-room "claude"))
            (should (member '("claude" "before-login") launch-reads))))))))

(defun romacs-agent-pool-auth-tests--delegate (root target)
  "Delegate a brief in ROOT using TARGET through the delegate handler."
  (romacs-agent-tools--delegate
   (romacs-mcp--object "cwd" root "task" "Read the fixture and report."
                       "title" "Read the fixture" "wake" "none"
                       "target" (romacs-mcp--object "provider" target))))

(defun romacs-agent-pool-auth-tests--task (session)
  "Read the status of the task originally started as SESSION."
  (romacs-agent-tools--task (romacs-agent-tools--known (romacs-agent-id session))))

(defun romacs-agent-pool-auth-tests--failed-task (session &rest providers)
  "Assert SESSION failed and its result explains why on each of PROVIDERS."
  (let* ((task (romacs-agent-pool-auth-tests--task session))
         (result (gethash "result" task)))
    (should (equal (gethash "status" task) "failed"))
    (should (string-search romacs-agent-pool-auth-tests--reason result))
    (dolist (provider providers)
      (should (string-match-p (concat "\\_<" (regexp-quote provider) "\\_>") result)))))

(ert-deftest romacs-agent-pool-auth-pinned-task-stays-and-explains-the-failure ()
  (romacs-agent-pool-auth-tests--with
    (romacs-agent-pool-auth-tests--transport '("claude2")
      (romacs-agent-pool-auth-tests--delegate root "claude2")
      (drain)
      (should (equal (mapcar #'cadr launches) '("claude2")))
      (romacs-agent-pool-auth-tests--failed-task (caar launches) "claude2"))))

;;; romacs-agent-pool-auth-tests.el ends here
