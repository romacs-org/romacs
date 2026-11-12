;;; gnosis-test-agent-review-provider.el --- Bound tutor ownership -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

(require 'ert)
(require 'gnosis-agent-review)
(require 'gnosis-test-helpers)

(defun gnosis-test-provider--question ()
  "Return one synthetic local question."
  '(:id "valve" :question "What opens?" :reference-answer "The valve."
    :rubric "Name the valve." :hints [] :parathema nil :tags [] :source "Synthetic valve"))

(defun gnosis-test-provider--start (&optional provider legacy)
  "Reserve with PROVIDER without deferred launch; LEGACY retains format 2."
  (cl-letf (((symbol-function 'gnosis-agent--schedule)
             (lambda (state)
               (when legacy
                 (let ((data (gnosis-agent-review--copy (gnosis-agent-review--data state))))
                   (cl-remf data :protocol)
                   (cl-remf data :proposal)
                   (gnosis-agent-review--commit state data (gnosis-review-state-remaining state)))))))
    (gnosis-agent-start-review-session
     :questions (vector (gnosis-test-provider--question)) :goal "Recall" :source "Synthetic"
     :provider provider)))

(defmacro gnosis-test-provider--with-review (&rest body)
  "Run BODY with an isolated native REVIEW and STATE."
  (declare (indent 0) (debug t))
  `(gnosis-test-with-db
     (let* ((gnosis-agent-review--providers nil)
            (gnosis-review-buffer-name (generate-new-buffer-name "*bound provider*"))
            (gnosis-review-centered nil)
            (buffers (buffer-list))
            (_status (gnosis-test-provider--start nil t))
            (state (gnosis-review--read-session))
            (review (gnosis-review--setup-buffer nil 'practice)))
       (unwind-protect
           (with-current-buffer review
             (setq gnosis-review--state state)
             ,@body)
         (dolist (buffer (seq-difference (buffer-list) buffers))
           (when (and (buffer-live-p buffer)
                      (or (eq buffer review)
                          (string-prefix-p "*Gnosis " (buffer-name buffer))))
             (kill-buffer buffer)))))))

(defun gnosis-test-provider--finish (request resolve _reject)
  "Resolve REQUEST through RESOLVE with a finite synthetic result."
  (funcall resolve
           (if (equal (plist-get request :phase) "evaluate")
               '(:verdict "fail" :explanation "The valve opens.")
             '(:agent-note "Finished." :context nil :remaining [] :done t)))
  #'ignore)

(ert-deftest gnosis-provider-start-binds-before-reentrant-schedule ()
  (gnosis-test-with-db
   (let ((gnosis-agent-review--providers nil) calls
         (gnosis-agent-review-function (lambda (&rest _) (ert-fail "Global provider used"))))
     (cl-letf (((symbol-function 'gnosis-agent--schedule)
                (lambda (state)
                  ;; A scheduler may reenter before start returns its identity.
                  (gnosis-agent-review--call-provider
                   state '(:phase "evaluate") #'ignore #'ignore))))
       (let ((status (gnosis-agent-start-review-session
                      :questions (vector (gnosis-test-provider--question))
                      :goal "Recall" :source "Synthetic"
                      :provider (lambda (request resolve _reject)
                                  (push request calls) (funcall resolve nil) #'ignore))))
         (should (equal "agent-review" (plist-get status :mode)))
         (should (= 1 (length calls))))))))

(ert-deftest gnosis-provider-completion-after-true-done-and-summary ()
  (gnosis-test-provider--with-review
   (let* (events observations requests
          (gnosis-agent-review-function (lambda (&rest _) (ert-fail "Wrong default")))
          (gnosis-practice-completed-hook
           (list (lambda (event)
                   ;; Keep assertions outside the subscriber's error boundary.
                   (push (list gnosis-review--running
                               (recursion-depth)
                               (derived-mode-p 'gnosis-agent-review-summary-mode)
                               (plist-get (gnosis-agent-results (plist-get event :session-id)) :status))
                         observations)
                   (push event events)))))
     (gnosis-agent-review-bind-provider
      (gnosis-review-state-session-id state) gnosis-db
      (lambda (request resolve reject)
        (push request requests)
        (gnosis-test-provider--finish request resolve reject)))
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda () (insert "The valve α\nopens") (gnosis-agent-eval-submit)))
               ((symbol-function 'gnosis-review--read-action)
                (let ((actions '(?o ?n))) (lambda (_) (pop actions)))))
       (gnosis-review--run-state review state))
     (should (equal observations '((nil 0 gnosis-agent-review-summary-mode "completed"))))
     (should (= 1 (length events)))
     (should (equal (plist-get (car events) :mode) "agent-review"))
     (should (eq (plist-get (car events) :connection) gnosis-db))
     (should (equal (mapcar (lambda (r) (plist-get r :phase)) (reverse requests))
                    '("evaluate" "adapt")))
     (let ((accepted (plist-get (car requests) :accepted)))
       (should (eq (plist-get accepted :override) t))
       (should (equal (plist-get accepted :outcome) "success"))
       (should (equal (plist-get (plist-get accepted :evaluation) :verdict) "fail")))
     (dolist (table '(themata extras scheduler-state review-events practice-events practice-encounters))
       (should-not (gnosis-select '* table)))
     (gnosis-review--show-summary state)
     (should (= 1 (length events)))
     (should-not (featurep 'hermes)))))

(ert-deftest gnosis-provider-replacement-retirement-and-late-delivery ()
  (gnosis-test-provider--with-review
   (let* ((id (gnosis-review-state-session-id state))
          (gnosis-agent-review-function (lambda (&rest _) (ert-fail "Fallback after loss")))
          resolve reject unbind cancel values failures replacement-refused
          (cancelled 0))
     (setq unbind
           (gnosis-agent-review-bind-provider
            id gnosis-db
            (lambda (_request done fail)
              (setq resolve done reject fail)
              (setq replacement-refused
                    (condition-case nil
                        (progn (gnosis-agent-review-bind-provider id gnosis-db #'ignore) nil)
                      (user-error t)))
              (lambda ()
                (cl-incf cancelled)
                (funcall done 'stale)
                (funcall fail "Late cancellation error")))))
     (setq cancel (gnosis-agent-review--call-provider
                   state nil (lambda (v) (push v values)) (lambda (v) (push v failures))))
     (should replacement-refused)
     (should-error (gnosis-agent-review-bind-provider id gnosis-db #'ignore) :type 'user-error)
     (funcall unbind)
     (funcall unbind)
     (funcall cancel)
     (funcall resolve 'late)
     (funcall reject "Late")
     (should (= cancelled 1))
     (should-not values)
     (should (equal failures '("Session provider was unbound")))
     (should-error (gnosis-agent-review--call-provider state nil #'ignore #'ignore) :type 'user-error)
     (let ((successor (gnosis-agent-review-bind-provider
                       id gnosis-db (lambda (_ done _reject) (funcall done 'successor) #'ignore))))
       (funcall unbind)
       (funcall (gnosis-agent-review--call-provider state nil (lambda (v) (push v values)) #'ignore))
       (should (equal values '(successor)))
       (funcall successor)))))

(ert-deftest gnosis-provider-retirement-during-start-and-cancel-reentry ()
  (gnosis-test-provider--with-review
   (let* ((id (gnosis-review-state-session-id state))
          unbind successor values failures (cancelled 0))
     (setq unbind
           (gnosis-agent-review-bind-provider
            id gnosis-db
            (lambda (_request resolve _reject)
              (funcall unbind)
              (funcall resolve 'retired)
              (lambda ()
                (cl-incf cancelled)
                (setq successor (gnosis-agent-review-bind-provider
                                 id gnosis-db
                                 (lambda (_ done _fail) (funcall done 'new) #'ignore)))
                (funcall (gnosis-agent-review--call-provider
                          state nil (lambda (v) (push v values)) #'ignore))))))
     (let ((cancel (gnosis-agent-review--call-provider
                    state nil (lambda (v) (push v values)) (lambda (v) (push v failures)))))
       (funcall cancel)
       (funcall unbind)
       (should (= cancelled 1))
       (should (equal values '(new)))
       (should (equal failures '("Session provider was unbound")))
       (funcall (gnosis-agent-review--call-provider
                 state nil (lambda (v) (push v values)) #'ignore))
       (should (equal values '(new new)))
       (funcall successor)))))

(ert-deftest gnosis-provider-synchronous-settlement-keeps-successor-owner ()
  (gnosis-test-provider--with-review
   (let* ((id (gnosis-review-state-session-id state))
          values successor-cancel old-unbind pending (cancelled 0))
     (setq old-unbind
           (gnosis-agent-review-bind-provider
            id gnosis-db
            (lambda (_ resolve _reject)
              (funcall resolve 'old)
              (lambda () (cl-incf cancelled) (funcall resolve 'late)))))
     (funcall
      (gnosis-agent-review--call-provider
       state nil
       (lambda (v)
         (push v values)
         (gnosis-agent-review-bind-provider
          id gnosis-db (lambda (_ resolve _reject) (setq pending resolve) #'ignore))
         (setq successor-cancel (gnosis-agent-review--call-provider
                                 state nil (lambda (v) (push v values)) #'ignore)))
       #'ignore))
     (funcall old-unbind)
     (should (= cancelled 1))
     (should-error (gnosis-agent-review-bind-provider id gnosis-db #'ignore) :type 'user-error)
     (funcall pending 'new)
     (funcall pending 'duplicate)
     (funcall successor-cancel)
     (should (equal values '(new old))))))

(ert-deftest gnosis-provider-start-failure-retires-binding ()
  (gnosis-test-with-db
   (let ((gnosis-agent-review--providers nil) captured
         (gnosis-agent-review-function (lambda (&rest _) (ert-fail "Failed start fallback"))))
     (cl-letf (((symbol-function 'gnosis-agent--schedule)
                (lambda (state) (setq captured state) (error "Synthetic schedule failure"))))
       (should-error (gnosis-agent-start-review-session
                      :questions (vector (gnosis-test-provider--question))
                      :source "Synthetic" :goal "Recall" :provider #'gnosis-test-provider--finish)))
     (should captured)
     (should-not (plist-get (gnosis-agent-review--provider-entry captured) :function))
     (should-error (gnosis-agent-review--call-provider captured nil #'ignore #'ignore) :type 'user-error))))

(ert-deftest gnosis-provider-exact-connection-session-and-default-isolation ()
  (gnosis-test-provider--with-review
   (let* ((id (gnosis-review-state-session-id state))
          (first-db gnosis-db) (first state) values unbind
          (gnosis-agent-review-function (lambda (&rest _) (ert-fail "Global default used"))))
     (setq unbind (gnosis-agent-review-bind-provider
                   id gnosis-db (lambda (_ done _fail) (funcall done 'first) #'ignore)))
     (gnosis-test-with-db
      (should-error (gnosis-agent-review-bind-provider id gnosis-db #'ignore) :type 'user-error)
      (gnosis-test-provider--start
       (lambda (_ done _fail) (funcall done 'second) #'ignore))
      (let ((second (gnosis-review--read-session)))
        (should-error (gnosis-agent-review--call-provider first nil #'ignore #'ignore) :type 'user-error)
        (funcall (gnosis-agent-review--call-provider second nil (lambda (v) (push v values)) #'ignore))
        (let ((gnosis-db first-db))
          (funcall (gnosis-agent-review--call-provider first nil (lambda (v) (push v values)) #'ignore)))
        (funcall (gnosis-agent-review--call-provider second nil (lambda (v) (push v values)) #'ignore))))
     (should (equal values '(second first second)))
     (gnosis-test-provider--start)
     (should-error (gnosis-agent-review-bind-provider id gnosis-db #'ignore) :type 'user-error)
     (funcall unbind)
     (gnosis-sqlite-close gnosis-db)
     (should-error (gnosis-agent-review-bind-provider id first-db #'ignore))
     (setq gnosis-db (gnosis-sqlite-open gnosis-test--db-file))
     (gnosis-db-init)
     (let* ((current (gnosis-review--read-session))
            (new-id (gnosis-review-state-session-id current)))
       (gnosis-agent-review-bind-provider
        new-id gnosis-db (lambda (_ done _fail) (funcall done 'reopened) #'ignore))
       (funcall unbind)
       (funcall (gnosis-agent-review--call-provider current nil (lambda (v) (push v values)) #'ignore))
       (should (eq (car values) 'reopened))))))

(ert-deftest gnosis-provider-paused-empty-adaptation-completes-only-on-resume ()
  (gnosis-test-provider--with-review
   (let (events
         (gnosis-practice-completed-hook nil))
     (setq gnosis-practice-completed-hook (list (lambda (event) (push event events))))
     (gnosis-agent-review-bind-provider (gnosis-review-state-session-id state) gnosis-db
                                        #'gnosis-test-provider--finish)
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda () (insert "Valve") (gnosis-agent-eval-submit)))
               ((symbol-function 'gnosis-review--read-action) (lambda (_) ?q)))
       (gnosis-review--run-state review state))
     (should-not events)
     (should-not (gnosis-review-state-remaining state))
     (should (equal (plist-get (gnosis-agent-review--data state) :phase) "adapt"))
     (cl-letf (((symbol-function 'read-multiple-choice) (lambda (&rest _) '(?p))))
       (gnosis-review--resume (gnosis-review--session-target)))
     (should-not events)
     (cl-letf (((symbol-function 'read-multiple-choice) (lambda (&rest _) '(?r))))
       (gnosis-review--resume (gnosis-review--session-target)))
     (should (= (length events) 1))
     (should (equal "agent-review" (plist-get (car events) :mode)))
     (should (equal "completed" (plist-get (gnosis-agent-status (gnosis-review-state-session-id state)) :status))))))

(ert-deftest gnosis-provider-native-bound-pause-resume-and-completion ()
  "Run an alternate tutor through real native input, override and resume."
  (skip-unless (not noninteractive))
  (gnosis-test-provider--with-review
   (let* ((stage 0) driver failure requests events notices
          (id (gnosis-review-state-session-id state))
          (gnosis-agent-review-function (lambda (&rest _) (ert-fail "Unrelated default")))
          (gnosis-practice-completed-hook
           (list (lambda (event)
                   (push event events)
                   (push (list gnosis-review--running (recursion-depth)
                               (derived-mode-p 'gnosis-agent-review-summary-mode)) notices)))))
     (gnosis-agent-review-bind-provider
      id gnosis-db
      (lambda (request resolve _reject)
        (push request requests)
        (let* ((phase (plist-get request :phase))
               (count (length (plist-get request :transcript)))
               (result (if (equal phase "evaluate")
                           (list :verdict (if (= count 0) "fail" "pass")
                                 :explanation "The synthetic valve opens.")
                         (if (= count 1)
                             (let ((next (copy-sequence (gnosis-test-provider--question))))
                               (plist-put next :id "followup")
                               (list :agent-note "Try the next question." :context nil
                                     :remaining (vector next) :done :false))
                           '(:agent-note "Finite block finished." :context nil :remaining [] :done t)))))
          ;; Both synchronous and deferred evaluation use the native ready event.
          (if (= count 0)
              (progn (funcall resolve result) #'ignore)
            (let ((timer (run-at-time 0.03 nil resolve result)))
              (lambda () (cancel-timer timer)))))))
     (unwind-protect
         (progn
           (setq driver
                 (run-at-time
                  0.01 0.01
                  (lambda ()
                    (condition-case err
                        (cond
                         ((and (memq stage '(0 2)) (derived-mode-p 'gnosis-agent-eval-mode))
                          (insert (if (= stage 0) "Unknown α\nPlease explain" "The valve."))
                          (cl-incf stage)
                          (setq unread-command-events (listify-key-sequence (kbd "C-c C-c"))))
                         ((and (memq stage '(1 3)) (bound-and-true-p gnosis-review-feedback-mode))
                          (should (string-match-p "Evaluator feedback:" (buffer-string)))
                          (setq unread-command-events
                                (listify-key-sequence (kbd (if (= stage 1) "o n" "q"))))
                          (cl-incf stage)))
                      ((error quit)
                       (setq failure err unread-command-events (listify-key-sequence (kbd "C-g"))))))))
           (with-timeout (10 (ert-fail "Bound native tutor timed out"))
             (gnosis-review--run-state review state))
           (should (= stage 4))
           (should-not events)
           (should (= 2 (gnosis-review-state-reviewed state)))
           (should-not (gnosis-review-state-remaining state))
           (should (equal "adapt" (plist-get (gnosis-agent-review--data state) :phase)))
           ;; Resume through the public scheduled entry, not a replacement batch.
           (setq unread-command-events (list ?r))
           (gnosis-agent-resume id)
           (with-timeout (10 (ert-fail "Bound resume did not complete"))
             (while (null events) (accept-process-output nil 0.05))))
       (when driver (cancel-timer driver)))
     (when failure (signal (car failure) (cdr failure)))
     (should (= 1 (length events)))
     (should (equal notices '((nil 0 gnosis-agent-review-summary-mode))))
     (should (equal (mapcar (lambda (r) (plist-get r :phase)) (reverse requests))
                    '("evaluate" "adapt" "evaluate" "adapt")))
     (let ((accepted (plist-get (nth 1 (reverse requests)) :accepted)))
       (should (eq (plist-get accepted :override) t))
       (should (equal (plist-get accepted :outcome) "success"))
       (should (equal (plist-get accepted :response) "Unknown α\nPlease explain")))
     (should (equal (plist-get (gnosis-agent-status id) :status) "completed"))
     (dolist (table '(themata extras scheduler-state scheduler-baseline review-events practice-events practice-encounters))
       (should-not (gnosis-select '* table)))
     (should-not (seq-some (lambda (feature) (string-prefix-p "hermes" (symbol-name feature))) features)))))

(ert-deftest gnosis-provider-summary-failure-does-not-notify ()
  (gnosis-test-provider--with-review
   (let (events (gnosis-practice-completed-hook nil))
     (setq gnosis-practice-completed-hook (list (lambda (event) (push event events))))
     (gnosis-agent-review-bind-provider (gnosis-review-state-session-id state) gnosis-db
                                        #'gnosis-test-provider--finish)
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda () (insert "Valve") (gnosis-agent-eval-submit)))
               ((symbol-function 'gnosis-review--read-action) (lambda (_) ?n))
               ((symbol-function 'gnosis-review--show-summary) (lambda (_) (error "Summary failed"))))
       (should-error (gnosis-review--run-state review state)))
     (should-not events))))

(provide 'gnosis-test-agent-review-provider)
;;; gnosis-test-agent-review-provider.el ends here
