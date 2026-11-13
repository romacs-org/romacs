;;; gnosis-test-agent-review.el --- Session-local tutor tests -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

(require 'ert)
(require 'gnosis-agent-review)
(require 'gnosis-agent-romacs)
(require 'gnosis-test-helpers)

(defun gnosis-test-local--question (&optional id)
  "Return fresh synthetic question with ID."
  (gnosis-agent-review--copy
   (list :id (or id "q1") :question "Explain the fictional α valve 🧠"
         :reference-answer "It opens.\nThen flow increases."
         :rubric "Require opening and increased flow; reject reverse causality."
         :hints ["Think about direction"] :parathema "Synthetic teaching material"
         :tags ["synthetic"] :source "Synthetic source: opening the valve increases flow.")))

(defun gnosis-test-local--reserve (&optional questions)
  "Reserve a legacy format-2 fixture without dispatching a native timer.
New public reservations use format 3; these tests retain the old controller."
  (cl-letf (((symbol-function 'gnosis-agent--schedule)
             (lambda (state)
               (let ((data (gnosis-agent-review--copy (gnosis-agent-review--data state))))
                 (cl-remf data :protocol)
                 (cl-remf data :proposal)
                 (gnosis-agent-review--commit state data (gnosis-review-state-remaining state))))))
    (gnosis-agent-start-review-session :questions (or questions (vector (gnosis-test-local--question)))
                                       :goal "Explain the synthetic valve" :source "Synthetic fixed scope"))
  (gnosis-review--read-session))

(defmacro gnosis-test-local--with-session (&rest body)
  "Run BODY with STATE and REVIEW using real disposable session storage."
  (declare (indent 0))
  `(gnosis-test-with-db
    (let* ((gnosis-review-buffer-name (generate-new-buffer-name "*local review test*"))
           (gnosis-review-centered nil)
           (state (gnosis-test-local--reserve))
           (review (gnosis-review--setup-buffer nil 'practice)))
      (unwind-protect
          (with-current-buffer review
            (setq gnosis-review--state state)
            ,@body)
        (when (buffer-live-p review) (kill-buffer review))))))

(defun gnosis-test-local--answer (state &optional action)
  "Answer STATE through native response submit, then select ACTION or Next.
Only recursive input and feedback choice are controlled; writers are real."
  (cl-letf (((symbol-function 'recursive-edit)
             (lambda ()
               (insert "It closes.\nFlow decreases α 🧠")
               (call-interactively (key-binding (kbd "C-c C-c")))) )
            ((symbol-function 'gnosis-review--read-action)
             (lambda (_context) (or action ?n))))
    (gnosis-agent-review--question-input state)))

(defun gnosis-test-local--evidence ()
  "Read collection and scheduler evidence, excluding local blobs."
  (mapcar (lambda (table) (gnosis-select '* table))
          '(themata extras thema-tag thema-links scheduler-state scheduler-baseline
		    review-events practice-events practice-encounters)))

(ert-deftest gnosis-test-local-reserve-v2-reopen-and-json ()
  (gnosis-test-local--with-session
   (should (= gnosis-db-version (caar (sqlite-select gnosis-db "PRAGMA user_version"))))
   (should (= 2 (plist-get (gnosis-review--state-data state) :version)))
   (should (equal (gnosis-test-local--evidence) '(nil nil nil nil nil nil nil nil nil)))
   (let ((id (gnosis-review-state-session-id state)))
     (should (equal (plist-get (gnosis-agent-status id) :mode) "agent-review"))
     (should (stringp (json-serialize (gnosis-agent-results id) :false-object :false :null-object nil)))
     (gnosis-sqlite-close gnosis-db)
     (setq gnosis-db (gnosis-sqlite-open gnosis-test--db-file))
     (gnosis-db-init)
     (should (equal (plist-get (gnosis-agent-current-practice) :session-id) id))
     (should (equal (gnosis-review-state-agent-review (gnosis-review--read-session))
                    (gnosis-review-state-agent-review state))))))

(ert-deftest gnosis-test-local-next-adapts-once-and-completes ()
  (gnosis-test-local--with-session
   (let* ((calls nil)
          (gnosis-agent-review-function
           (lambda (request resolve _reject)
             (push request calls)
             (pcase (plist-get request :phase)
               ("evaluate" (funcall resolve '(:verdict "fail" :explanation "Reversed direction: opening increases flow.")))
               ("adapt" (should (equal (plist-get (plist-get request :accepted) :outcome) "failure"))
                (funcall resolve '(:agent-note "Revisit after another topic." :context (:topic "valve")
					       :remaining [] :done t))))
             #'ignore)))
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda () (insert "It closes.") (gnosis-agent-eval-submit)))
               ((symbol-function 'gnosis-review--read-action) (lambda (_) ?n))
               ((symbol-function 'read-multiple-choice)
                (lambda (&rest _) (ert-fail "Next must not ask a second confirmation"))))
       (gnosis-agent-review--run state))
     (should (equal (mapcar (lambda (r) (plist-get r :phase)) (reverse calls)) '("evaluate" "adapt")))
     (should-not (gnosis-review--work-p state))
     (should (= 1 (gnosis-review-state-reviewed state)))
     (should (equal (plist-get (gnosis-agent-status (gnosis-review-state-session-id state)) :status) "completed"))
     (should (equal (gnosis-test-local--evidence) '(nil nil nil nil nil nil nil nil nil)))
     (should (equal (plist-get (gnosis-review--completion-event state) :mode)
                    "agent-review")))))

(ert-deftest gnosis-test-local-override-adaptation-and-immutable-followup ()
  (gnosis-test-local--with-session
   (let* ((calls nil) (choices '(?o ?n))
          (followup (gnosis-test-local--question "repair"))
          (gnosis-agent-review-function
           (lambda (request resolve _reject)
             (push request calls)
             (if (equal (plist-get request :phase) "evaluate")
                 (funcall resolve '(:verdict "fail" :explanation "Missing direction."))
               (should (equal (plist-get (plist-get request :accepted) :outcome) "success"))
               (should (eq t (plist-get (plist-get request :accepted) :override)))
               (funcall resolve (list :agent-note "Short repair explanation." :context '(:repair t)
                                      :remaining (vector followup) :done :false)))
             #'ignore)))
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda () (insert "Actual answer α") (gnosis-agent-eval-submit)))
               ((symbol-function 'gnosis-review--read-action) (lambda (_) (pop choices))))
       (gnosis-agent-review--question-input state))
     (should (string-match-p "Evaluator feedback:" (buffer-string)))
     (should (equal (plist-get (gnosis-agent-review--data state) :phase) "adapt"))
     (should (gnosis-review--work-p state))
     (gnosis-agent-review--adapt state t)
     (should (equal (gnosis-review-state-remaining state) '("repair")))
     (should (equal (gnosis-agent-review--find (gnosis-agent-review--data state) "q1")
                    (gnosis-test-local--question)))
     (should (equal (plist-get (gnosis-agent-review--data state) :context) '(:repair t))))))

(ert-deftest gnosis-test-local-pause-draft-reopen-retry ()
  (gnosis-test-local--with-session
   (let ((gnosis-agent-review-function (lambda (_request _resolve reject)
                                         (funcall reject "Offline") #'ignore)))
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda ()
                  (insert "Unfinished multiline\nα draft")
                  (gnosis-agent-eval-submit)
                  (should-not buffer-read-only)
                  (signal 'quit nil))))
       (should (eq 'paused (condition-case nil (gnosis-agent-review--question-input state)
                             (quit 'paused))))))
   (let* ((id (gnosis-review-state-session-id state))
          (data (gnosis-agent-review--data (gnosis-agent--session id))))
     (should (equal (plist-get (plist-get data :current) :response) "Unfinished multiline\nα draft"))
     (should (equal (plist-get (aref (plist-get data :attempts) 0) :error) "Offline"))
     (should (= 0 (length (plist-get data :encounters))))
     (gnosis-sqlite-close gnosis-db)
     (setq gnosis-db (gnosis-sqlite-open gnosis-test--db-file))
     (gnosis-db-init)
     (setq state (gnosis-review--read-session) gnosis-review--state state)
     (let ((gnosis-agent-review-function
            (lambda (request resolve _reject)
              (should (equal (plist-get request :response) "Unfinished multiline\nα draft"))
              (funcall resolve '(:verdict "pass" :explanation "Accepted essentials.")) #'ignore)))
       (cl-letf (((symbol-function 'recursive-edit)
                  (lambda ()
                    (should (equal (buffer-string) "Unfinished multiline\nα draft"))
                    (gnosis-agent-eval-submit)))
                 ((symbol-function 'gnosis-review--read-action) (lambda (_) ?n)))
         (gnosis-agent-review--question-input state)))
     (should (= 1 (gnosis-review-state-reviewed state))))))

(ert-deftest gnosis-test-local-empty-adapt-resume-replace-cancel ()
  (dolist (operation '(resume replace cancel))
    (gnosis-test-local--with-session
     (let ((gnosis-agent-review-function
            (lambda (_request resolve _reject)
              (funcall resolve '(:verdict "pass" :explanation "Correct.")) #'ignore)))
       (gnosis-test-local--answer state))
     (let ((id (gnosis-review-state-session-id state)))
       (should-not (gnosis-review-state-remaining state))
       (should (gnosis-review--work-p state))
       (should (equal (plist-get (gnosis-agent-status id) :status) "unfinished"))
       (pcase operation
         ('resume
          (let (scheduled)
            (cl-letf (((symbol-function 'gnosis-agent--schedule)
                       (lambda (s) (setq scheduled s))))
              (gnosis-agent-resume id))
            (should (gnosis-review--work-p scheduled))))
         ('replace
          (gnosis-test-local--reserve (vector (gnosis-test-local--question "successor")))
          (should (equal (plist-get (gnosis-agent-status id) :status) "cancelled")))
         ('cancel
          (should (equal (plist-get (gnosis-agent-cancel id) :status) "cancelled"))
          (should-not (gnosis-review--read-session))))
       (should (= 1 (length (plist-get (gnosis-agent-review--data (gnosis-agent--session id)) :encounters))))))))

(ert-deftest gnosis-test-local-save-exact-current-only-idempotent ()
  (gnosis-test-local--with-session
   (let* ((id (gnosis-review-state-session-id state))
          (receipt (gnosis-agent-review-save-question id "q1")))
     (should (= 1 (length (gnosis-select '* 'themata))))
     (should (equal receipt (gnosis-agent-review-save-question id "q1")))
     (should-not (gnosis-select '* 'review-events))
     (should-not (gnosis-select '* 'practice-events))
     (let ((gnosis-review--running id))
       (should-error (gnosis-agent-review-save-question id "q1") :type 'user-error))
     (gnosis-test-local--reserve (vector (gnosis-test-local--question "next")))
     (let ((before (gnosis-review--session-target)))
       (should-error (gnosis-agent-review-save-question id "q1") :type 'user-error)
       (should (equal before (gnosis-review--session-target)))))))

(ert-deftest gnosis-test-local-save-rollback-and-drift ()
  (gnosis-test-local--with-session
   (let ((id (gnosis-review-state-session-id state))
         (before (gnosis-review--session-target)))
     (cl-letf (((symbol-function 'gnosis-review--save-history) (lambda (_) (error "Injected receipt failure"))))
       (should-error (gnosis-agent-review-save-question id "q1")))
     (should (equal before (gnosis-review--session-target)))
     (should-not (gnosis-select '* 'themata))
     (let ((receipt (gnosis-agent-review-save-question id "q1")))
       (gnosis-sqlite-execute gnosis-db "UPDATE themata SET keimenon = ? WHERE id = ?"
                              (list "Changed" (string-to-number (plist-get receipt :thema-id))))
       (should-error (gnosis-agent-review-save-question id "q1") :type 'user-error)
       (should (= 1 (length (gnosis-select '* 'themata))))))))

(ert-deftest gnosis-test-local-plan-validation-and-v1-compatibility ()
  (gnosis-test-local--with-session
   (let* ((data (gnosis-agent-review--data state))
          (changed (gnosis-test-local--question))
          (before (copy-tree data t)))
     (setf (plist-get changed :question) "Different prompt")
     (should-error (gnosis-agent-review--plan data (list :agent-note "" :context nil
                                                         :remaining (vector changed) :done :false)))
     (should-error (gnosis-agent-review--plan data '(:agent-note "" :context nil :remaining [] :done :false)))
     (should-error (gnosis-agent-review--plan data '(:agent-note "" :context nil :remaining [] :done t :done t)))
     (should (equal before data))
     (should-error (gnosis-agent-review--text (make-string 32769 ?a)))
     (let* ((ordinary (gnosis-review-state-create :mode 'practice :session-id "v1" :remaining '(42)))
            (encoded (gnosis-review--state-data ordinary)))
       (should (= 1 (plist-get encoded :version)))
       (should-not (plist-member encoded :agent-review))
       (should (equal '(42) (gnosis-review-state-remaining (gnosis-review--decode-session encoded))))))))

(ert-deftest gnosis-test-local-adapt-duplicate-stale-and-reentrant-cancel ()
  (gnosis-test-local--with-session
   (let ((gnosis-agent-review-function
          (lambda (_request resolve _reject)
            (funcall resolve '(:verdict "pass" :explanation "Correct.")) #'ignore)))
     (gnosis-test-local--answer state))
   (let* ((stale nil) (count 0)
          (plan '(:agent-note "Done." :context nil :remaining [] :done t))
          (gnosis-agent-review-function
           (lambda (_request resolve reject)
             (setq stale resolve)
             (funcall resolve plan)
             (funcall resolve '(:invalid t))
             (funcall reject "Duplicate error")
             (lambda () (cl-incf count) (funcall resolve '(:invalid t))))))
     (gnosis-agent-review--adapt state t)
     (should (= 1 count))
     (let ((before (gnosis-review--session-target)))
       (funcall stale plan)
       (should (equal before (gnosis-review--session-target))))
     (should-not (gnosis-review--work-p state)))))

(ert-deftest gnosis-test-local-adapt-failure-explicit-continue ()
  (gnosis-test-local--with-session
   (let ((gnosis-agent-review-function
          (lambda (_request resolve _reject)
            (funcall resolve '(:verdict "pass" :explanation "Correct.")) #'ignore)))
     (gnosis-test-local--answer state))
   (let ((gnosis-agent-review-function (lambda (_request _resolve reject)
                                         (funcall reject "Unavailable") #'ignore))
         (choices 0))
     (cl-letf (((symbol-function 'read-multiple-choice)
                (lambda (prompt _) (should (equal prompt "Unavailable"))
                  (cl-incf choices) '(?c "continue"))))
       (gnosis-agent-review--adapt state t))
     (should (= choices 1))
     (should (= 1 (gnosis-review-state-reviewed state)))
     (should-not (gnosis-review--work-p state)))))

(ert-deftest gnosis-test-local-romacs-unicode-and-duplicate-fields ()
  (gnosis-test-local--with-session
   (gnosis-agent-review--present state)
   (let* ((request (gnosis-agent-review--request state "evaluate" "request" "Greek α\n🧠"))
          (prompt (gnosis-agent-romacs--review-prompt request))
          (outer (json-parse-string (json-serialize (list :prompt prompt)) :object-type 'plist)))
     (should (string-match-p "Greek α" (plist-get outer :prompt)))
     (should (string-match-p "🧠" (plist-get outer :prompt)))
     (should-error (gnosis-agent-romacs--parse-review "{\"verdict\":\"pass\",\"verdict\":\"fail\"}"))
     (should-error (gnosis-agent-romacs--parse-review "{} trailing")))))

(ert-deftest gnosis-test-local-acceptance-and-plan-rollback ()
  (gnosis-test-local--with-session
   (let ((gnosis-agent-review-function
          (lambda (_request resolve _reject)
            (funcall resolve '(:verdict "pass" :explanation "Correct.")) #'ignore))
         pending)
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda () (insert "Actual response") (gnosis-agent-eval-submit)))
               ((symbol-function 'gnosis-review-actions)
                (lambda (_success _id result) (setq pending result))))
       (gnosis-agent-review--question-input state))
     (let ((before (gnosis-review--session-target)))
       (cl-letf (((symbol-function 'gnosis-review--save-history)
                  (lambda (_) (error "Injected acceptance failure"))))
         (should-error (gnosis-review-result "q1" t pending)))
       (should (equal before (gnosis-review--session-target)))
       (should (= 0 (gnosis-review-state-reviewed state))))
     (gnosis-review-result "q1" t pending)
     (should-error (gnosis-review-result "q1" t pending) :type 'user-error)
     (let* ((before (gnosis-review--session-target))
            (plan '(:agent-note "Done" :context nil :remaining [] :done t))
            (next (gnosis-agent-review--plan (gnosis-agent-review--data state) plan)))
       (cl-letf (((symbol-function 'gnosis-review--save-history)
                  (lambda (_) (error "Injected plan failure"))))
         (should-error (gnosis-agent-review--commit state next nil)))
       (should (equal before (gnosis-review--session-target)))
       (should (equal "adapt" (plist-get (gnosis-agent-review--data state) :phase)))))))

(ert-deftest gnosis-test-local-checkpoint-failure-retains-native-draft ()
  (gnosis-test-local--with-session
   (let ((save (symbol-function 'gnosis-review--save-history))
         fail draft)
     (cl-letf (((symbol-function 'gnosis-review--save-history)
                (lambda (s) (if fail (error "Injected checkpoint failure") (funcall save s))))
               ((symbol-function 'recursive-edit)
                (lambda ()
                  (setq draft (current-buffer) fail t)
                  (insert "Never lose this\nα draft")
                  (signal 'quit nil))))
       (should-error (gnosis-agent-review--question-input state)))
     (should (buffer-live-p draft))
     (with-current-buffer draft
       (should (equal (buffer-string) "Never lose this\nα draft"))
       (should-not buffer-read-only)
       (should-not gnosis-agent-eval--context)
       (set-buffer-modified-p nil)
       (kill-buffer))
     (should (= 0 (gnosis-review-state-reviewed state)))
     (should-not (gnosis-select '* 'themata)))))

(ert-deftest gnosis-test-local-adaptation-cancel-reentry-retires-continuation ()
  (gnosis-test-local--with-session
   (let ((gnosis-agent-review-function
          (lambda (_request resolve _reject)
            (funcall resolve '(:verdict "pass" :explanation "Correct.")) #'ignore)))
     (gnosis-test-local--answer state))
   (let ((gnosis-agent-review-function
          (lambda (_request resolve _reject)
            (funcall resolve '(:agent-note "Obsolete plan" :context nil :remaining [] :done t))
            (lambda ()
              (let ((data (gnosis-agent-review--copy (gnosis-agent-review--data state))))
                (setf (plist-get data :agent-note) "Successor owns this revision")
                (gnosis-agent-review--commit state data nil))))))
     (should-error (gnosis-agent-review--adapt state t) :type 'user-error)
     (should (equal (plist-get (gnosis-agent-review--data state) :agent-note)
                    "Successor owns this revision"))
     (should (equal (plist-get (gnosis-agent-review--data state) :phase) "adapt")))))

(ert-deftest gnosis-test-local-stale-evaluation-db-and-buffer ()
  (dolist (retire '(database buffer))
    (gnosis-test-local--with-session
     (let* (resolve response failure
		    (original gnosis-db)
		    (gnosis-agent-review-function (lambda (_request yes _reject) (setq resolve yes) #'ignore)))
       (cl-letf (((symbol-function 'recursive-edit)
                  (lambda ()
                    (setq response (current-buffer))
                    (insert "Actual response")
                    (gnosis-agent-eval-submit)
                    (if (eq retire 'database)
                        (setq gnosis-db nil)
                      (with-current-buffer review (text-mode) (insert "Successor text")))
                    (funcall resolve '(:verdict "pass" :explanation "Late"))
                    (setq failure (plist-get gnosis-agent-eval--context :result)))))
         (should-error (gnosis-agent-review--question-input state)))
       (should-not failure)
       (setq gnosis-db original)
       (should (= 0 (length (plist-get (gnosis-agent-review--data (gnosis-review--read-session)) :encounters))))
       (when (eq retire 'buffer)
         (with-current-buffer review (should (string-suffix-p "Successor text" (buffer-string)))))
       (when (buffer-live-p response)
         (with-current-buffer response (set-buffer-modified-p nil) (kill-buffer)))))))

(ert-deftest gnosis-test-local-timeout-pause-and-source-copy-isolation ()
  (gnosis-test-local--with-session
   (let* ((question (gnosis-test-local--question))
          (input (vector question))
          (reserved (gnosis-test-local--reserve input)))
     (aset (plist-get question :question) 0 ?X)
     (should-not (string-prefix-p "X" (plist-get (gnosis-agent-review--find
                                                  (gnosis-agent-review--data reserved) "q1") :question)))
     ;; Adopt the deliberately replaced native reservation.
     (setq state reserved gnosis-review--state state))
   (let ((gnosis-agent-review-function
          (lambda (_request resolve _reject)
            (funcall resolve '(:verdict "pass" :explanation "Correct.")) #'ignore)))
     (gnosis-test-local--answer state))
   (let ((gnosis-agent-eval-timeout 0)
         (gnosis-agent-review-function (lambda (&rest _) #'ignore)))
     (cl-letf (((symbol-function 'read-multiple-choice)
                (lambda (prompt _) (should (equal prompt "Adaptation timed out")) '(?p "pause"))))
       (catch 'review-loop (gnosis-agent-review--adapt state t)))
     (should (gnosis-review--work-p state))
     (should (= 1 (gnosis-review-state-reviewed state))))))

(ert-deftest gnosis-test-local-native-answer-source-override-followup-save-resume ()
  "Exercise the real command loop, native Next and paused explicit saving."
  (skip-unless (not noninteractive))
  (gnosis-test-local--with-session
   (let* ((stage 0) failure driver requests
          (origin (current-buffer))
          (gnosis-agent-review-function
           (lambda (request resolve _reject)
             (push request requests)
             (let* ((phase (plist-get request :phase))
                    (first (= (length (plist-get request :transcript)) 0))
                    (result
                     (if (equal phase "evaluate")
                         (list :verdict (if first "fail" "pass")
                               :explanation (if first "Direction is reversed." "Correct repair."))
                       (if (= (length (plist-get request :transcript)) 1)
                           (list :agent-note "Repair the direction." :context '(:repair t)
                                 :remaining (vector (gnosis-test-local--question "repair")) :done :false)
                         '(:agent-note "Finished synthetic practice." :context nil :remaining [] :done t))))
                    (timer (run-at-time 0.02 nil resolve result)))
               (lambda () (cancel-timer timer)))))
          (_driver
           (setq driver
                 (run-at-time
                  0.01 0.01
                  (lambda ()
                    (condition-case err
                        (let ((context (buffer-local-value 'gnosis-agent-eval--review-context origin)))
                          (cond
                           ((and (= stage 2) (bound-and-true-p gnosis-link-view-mode)
                                 (> (recursion-depth) 0))
                            (should (string-match-p "Synthetic source" (buffer-string)))
                            (setq stage 3 unread-command-events
                                  (listify-key-sequence (kbd "C-c C-c o n"))))
                           ((and (memq stage '(1 4))
                                 (bound-and-true-p gnosis-review-feedback-mode))
                            (setq unread-command-events
                                  (listify-key-sequence (kbd (if (= stage 1) "v" "q"))))
                            (cl-incf stage))
                           ((and context (eq (current-buffer) (plist-get context :buffer))
                                 (= (recursion-depth) (1+ (plist-get context :depth)))
                                 (gnosis-agent-eval--valid-p context))
                            (cond
                             ((memq stage '(0 3))
                              (insert (if (= stage 0) "It closes.\nFlow decreases."
                                        "It opens.\nFlow increases."))
                              (cl-incf stage)
                              (execute-kbd-macro (kbd "C-c C-c")))))))
                      ((error quit)
                       (setq failure err unread-command-events
                             (listify-key-sequence (kbd "C-g"))))))))))
     (unwind-protect
         (with-timeout (15 (ert-fail "Native local-review journey timed out"))
           (gnosis-review--run-state origin state))
       (cancel-timer driver))
     (when failure (signal (car failure) (cdr failure)))
     (should (= stage 5))
     (should (= 2 (gnosis-review-state-reviewed state)))
     (should (equal (plist-get (gnosis-agent-review--data state) :phase) "adapt"))
     (should-not (gnosis-review-state-remaining state))
     (should-not (gnosis-select '* 'themata))
     (let* ((id (gnosis-review-state-session-id state))
            (receipt (gnosis-agent-review-save-question id "repair")))
       (should (stringp (plist-get receipt :thema-id)))
       (gnosis-sqlite-close gnosis-db)
       (setq gnosis-db (gnosis-sqlite-open gnosis-test--db-file))
       (gnosis-db-init)
       (setq unread-command-events (list ?r))
       (with-timeout (10 (ert-fail "Native adaptation recovery timed out"))
         (gnosis-review--resume (gnosis-review--session-target)))
       (should (equal (plist-get (gnosis-agent-status id) :status) "completed"))
       (should (equal receipt (gnosis-agent-review-save-question id "repair")))
       (should-not (gnosis-select '* 'review-events))
       (should-not (gnosis-select '* 'practice-events)))
     (let ((adapt (seq-find (lambda (r) (equal "adapt" (plist-get r :phase)))
                            (reverse requests))))
       (should (eq t (plist-get (plist-get adapt :accepted) :override)))
       (should (eq t (plist-get (plist-get adapt :accepted) :source-viewed)))
       (should (equal (plist-get (plist-get adapt :accepted) :outcome) "success"))))))

(ert-deftest gnosis-test-local-request-isolation-and-collection-refusal ()
  (gnosis-test-local--with-session
   (gnosis-agent-review--present state)
   (let* ((data (gnosis-agent-review--copy (gnosis-agent-review--data state)))
          (request (gnosis-agent-review--request state "evaluate" "request" "Answer")))
     (aset (plist-get request :goal) 0 ?X)
     (aset (plist-get request :source) 0 ?X)
     (aset (plist-get (plist-get request :question) :question) 0 ?X)
     (should (equal data (gnosis-agent-review--data state)))
     (let* ((plan (list :agent-note (copy-sequence "Teaching note") :context nil
                        :remaining [] :done t))
            (next (gnosis-agent-review--plan data plan)))
       (aset (plist-get (gnosis-agent-review--find next "q1") :question) 0 ?X)
       (aset (plist-get plan :agent-note) 0 ?X)
       (should (equal (plist-get next :agent-note) "Teaching note"))
       (should (equal data (gnosis-agent-review--data state)))))
   (let ((gnosis-agent-review-function
          (lambda (_request resolve _reject)
            (let ((value (list :verdict "pass" :explanation (copy-sequence "Correct."))))
              (funcall resolve value)
              (aset (plist-get value :explanation) 0 ?X))
            #'ignore)))
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda () (insert "Actual answer") (gnosis-agent-eval-submit)))
               ((symbol-function 'gnosis-review-actions)
                (lambda (success id result)
                  (let ((gnosis-review--feedback (list :result result)))
                    (should-not (gnosis-review--collection-action-p)))
                  (dolist (choice '(?e ?f ?s ?d))
                    (should-error (gnosis-review--collection-action-check choice result)
                                  :type 'user-error))
                  (should-error (gnosis-review-action--edit success id result) :type 'user-error)
                  (should-error (gnosis-review-action--suspend success id result) :type 'user-error))))
       (gnosis-agent-review--question-input state)))
   (should (= 0 (gnosis-review-state-reviewed state)))
   (should (equal "Correct." (plist-get (plist-get (plist-get (gnosis-agent-review--data state)
                                                              :current) :evaluation) :explanation)))
   (should-not (gnosis-select '* 'themata))))

(ert-deftest gnosis-test-local-submit-rollback-and-cancel-unsubmitted-draft ()
  (gnosis-test-local--with-session
   (let ((save (symbol-function 'gnosis-review--save-history))
         fail called)
     (let ((gnosis-agent-review-function (lambda (&rest _) (setq called t) #'ignore)))
       (cl-letf (((symbol-function 'gnosis-review--save-history)
                  (lambda (s) (if fail (error "Injected submit failure") (funcall save s))))
                 ((symbol-function 'recursive-edit)
                  (lambda ()
                    (insert "Draft not submitted α")
                    (let ((before (gnosis-review--session-target)))
                      (setq fail t)
                      (should-error (gnosis-agent-eval-submit))
                      (setq fail nil)
                      (should (equal before (gnosis-review--session-target))))
                    (should-not called)
                    (should-not buffer-read-only)
                    (gnosis-agent-eval-cancel)
                    (should (equal "Draft not submitted α"
                                   (plist-get (plist-get (gnosis-agent-review--data
                                                          (gnosis-review--read-session)) :current) :response)))
                    (signal 'quit nil))))
         (condition-case nil (gnosis-agent-review--question-input state) (quit nil))))
     (should (= 0 (length (plist-get (gnosis-agent-review--data state) :attempts)))))))

(ert-deftest gnosis-test-local-public-deferred-start-combined-completion ()
  (gnosis-test-with-db
   (let* ((gnosis-agent--launches nil)
          (gnosis-review-buffer-name (generate-new-buffer-name "*local launch*"))
          (gnosis-review-centered nil)
          (gnosis-agent-review-function
           (lambda (request resolve reject)
             (if (equal (plist-get request :phase) "evaluate")
                 (funcall resolve '(:api-version 2 :evaluation (:verdict "pass" :explanation "Correct.")
                                    :adjustment nil))
               (funcall reject "Unexpected post-Next adaptation"))
             #'ignore))
          (status (gnosis-agent-start-review-session
                   :questions (vector (gnosis-test-local--question))
                   :goal "Synthetic goal" :source "Synthetic scope"))
          (id (plist-get status :session-id)))
     (unwind-protect
         (progn
           (should (equal (plist-get status :status) "pending"))
           (should (= 0 (plist-get status :accepted)))
           (cancel-timer (plist-get (car gnosis-agent--launches) :timer))
           (cl-letf (((symbol-function 'recursive-edit)
                      (lambda () (insert "Actual answer") (gnosis-agent-eval-submit)))
                     ((symbol-function 'gnosis-review--read-action) (lambda (_) ?n))
                     ((symbol-function 'read-multiple-choice) (lambda (&rest _) '(?p "pause"))))
             (gnosis-agent--launch (car gnosis-agent--launches)))
           (should (equal (plist-get (gnosis-agent-status id) :phase) "done"))
           (should (equal (plist-get (gnosis-agent-status id) :status) "completed"))
           (should (= 1 (plist-get (gnosis-agent-status id) :accepted)))
           (should (equal (gnosis-test-local--evidence) '(nil nil nil nil nil nil nil nil nil))))
       (dolist (record (copy-sequence gnosis-agent--launches)) (gnosis-agent--release record))
       (when-let* ((buffer (get-buffer gnosis-review-buffer-name))) (kill-buffer buffer))))))

(defun gnosis-test-local--cancel-settled (native)
  "Preserve settled evidence through cancel, pause and reopen using NATIVE input."
  (dolist (verdict '("pass" "fail" "ungradable" "error"))
    (gnosis-test-local--with-session
     (let* (before driver failure (stage 0)
		   (gnosis-agent-review-function
		    (lambda (_request resolve reject)
                      ;; User cancellation already queued before settlement
                      ;; wins over automatic presentation, without losing it.
                      (when native
                        (setq unread-command-events (listify-key-sequence (kbd "C-c C-k"))))
		      (if (equal verdict "error") (funcall reject "Offline")
			(funcall resolve (list :verdict verdict :explanation "Original explanation α")))
                      (when native
                        (setq before (gnosis-agent-review--copy
                                      (plist-get (gnosis-agent-review--data state) :attempts))
                              stage 2))
		      #'ignore))
		   (step
		    (lambda ()
		      (pcase stage
			(0 (insert "Actual multiline\nresponse α")
			   (setq stage 1)
			   (if native
                               (setq unread-command-events (listify-key-sequence (kbd "C-c C-c")))
			     (gnosis-agent-eval-submit)))
			(1 (when (and (> (length (plist-get (gnosis-agent-review--data state) :attempts)) 0)
				      (not (plist-get gnosis-agent-eval--context :attempt)))
			     (setq before (gnosis-agent-review--copy
					   (plist-get (gnosis-agent-review--data state) :attempts))
				   stage 2)
			     (if native
				 (setq unread-command-events (listify-key-sequence (kbd "C-c C-k")))
                               (gnosis-agent-eval-cancel))))
			(2 (when (and (not buffer-read-only)
				      (not (plist-get (plist-get (gnosis-agent-review--data state) :current)
						      :evaluation)))
			     (setq stage 3)
			     (if native
				 (setq unread-command-events (listify-key-sequence (kbd "C-g")))
                               (signal 'quit nil))))))))
       (unwind-protect
           (if native
               (progn
                 (setq driver
                       (run-at-time
                        0.02 0.02
                        (lambda ()
                          (when (and (eq major-mode 'gnosis-agent-eval-mode) (> (recursion-depth) 0))
                            (condition-case err (funcall step)
                              ((error quit) (setq failure err unread-command-events
                                                  (listify-key-sequence (kbd "C-g")))))))))
                 (with-timeout (10 (ert-fail "Native cancellation did not complete"))
                   (condition-case nil (gnosis-agent-review--question-input state) (quit nil))))
             (cl-letf (((symbol-function 'recursive-edit)
                        (lambda () (funcall step) (funcall step) (funcall step))))
               (condition-case nil (gnosis-agent-review--question-input state) (quit nil))))
         (when driver (cancel-timer driver)))
       (when failure (signal (car failure) (cdr failure)))
       (should (= stage 3))
       (should (= 1 (length before)))
       (gnosis-sqlite-close gnosis-db)
       (setq gnosis-db (gnosis-sqlite-open gnosis-test--db-file))
       (gnosis-db-init)
       (setq state (gnosis-review--read-session) gnosis-review--state state)
       (should (equal before (plist-get (gnosis-agent-review--data state) :attempts)))
       (should-not (plist-get (plist-get (gnosis-agent-review--data state) :current) :evaluation))
       ;; Retrying the reopened response appends evidence, never replaces it.
       (let ((gnosis-agent-review-function
              (lambda (_request resolve _reject)
                (funcall resolve '(:verdict "pass" :explanation "Retry result")) #'ignore)))
         (cl-letf (((symbol-function 'recursive-edit)
                    (lambda ()
                      (should (equal (buffer-string) "Actual multiline\nresponse α"))
                      (gnosis-agent-eval-submit)))
                   ((symbol-function 'gnosis-review--read-action) (lambda (_) ?n)))
           (gnosis-agent-review--question-input state)))
       (should (equal (aref before 0) (aref (plist-get (gnosis-agent-review--data state) :attempts) 0)))
       (should (= 2 (length (plist-get (gnosis-agent-review--data state) :attempts))))))))

(ert-deftest gnosis-test-local-cancel-settled-pause-reopen ()
  (gnosis-test-local--cancel-settled nil))

(ert-deftest gnosis-test-local-native-cancel-settled-pause-reopen ()
  (skip-unless (not noninteractive))
  (gnosis-test-local--cancel-settled t))

(ert-deftest gnosis-test-local-cancel-unsubmitted-followup-preserves-adaptation ()
  (gnosis-test-local--with-session
   (let ((gnosis-agent-review-function
          (lambda (_request resolve _reject)
            (funcall resolve '(:verdict "pass" :explanation "Correct.")) #'ignore)))
     (gnosis-test-local--answer state))
   (let ((gnosis-agent-review-function
          (lambda (_request resolve _reject)
            (funcall resolve (list :agent-note "Next" :context nil
                                   :remaining (vector (gnosis-test-local--question "q2")) :done :false))
            #'ignore)))
     (gnosis-agent-review--adapt state t))
   (let ((before (gnosis-agent-review--copy (plist-get (gnosis-agent-review--data state) :attempts))))
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda ()
                  (insert "Unsubmitted follow-up")
                  (gnosis-agent-eval-cancel)
                  (signal 'quit nil))))
       (condition-case nil (gnosis-agent-review--question-input state) (quit nil)))
     (gnosis-sqlite-close gnosis-db)
     (setq gnosis-db (gnosis-sqlite-open gnosis-test--db-file))
     (gnosis-db-init)
     (should (equal before (plist-get (gnosis-agent-review--data (gnosis-review--read-session)) :attempts))))))

(ert-deftest gnosis-test-local-cancel-pending-late-and-reentrant-retry ()
  (dolist (reentrant '(nil t))
    (gnosis-test-local--with-session
     (let* (late (calls 0) (cancels 0)
		 (gnosis-agent-review-function
		  (lambda (_request resolve reject)
		    (cl-incf calls)
		    (if (= calls 1)
			(progn
			  (setq late resolve)
			  (lambda ()
			    (cl-incf cancels)
			    (funcall resolve '(:verdict "fail" :explanation "Cancelled callback"))
			    (funcall reject "Cancelled rejection")
			    (when reentrant (gnosis-agent-eval-submit))))
                      (funcall resolve '(:verdict "pass" :explanation "Successor result"))
                      #'ignore))))
       (cl-letf (((symbol-function 'recursive-edit)
                  (lambda ()
                    (insert "Pending response")
                    (gnosis-agent-eval-submit)
                    (gnosis-agent-eval-cancel)
                    (unless reentrant
                      (let ((before (gnosis-agent-review--copy
                                     (plist-get (gnosis-agent-review--data state) :attempts))))
                        (gnosis-agent-eval-cancel)
                        (should (equal before (plist-get (gnosis-agent-review--data state) :attempts))))
                      (gnosis-agent-eval-submit))
                    (let ((before (gnosis-review--session-target)))
                      (funcall late '(:verdict "fail" :explanation "Late old result"))
                      (should (equal before (gnosis-review--session-target))))))
                 ((symbol-function 'gnosis-review--read-action) (lambda (_) ?n)))
         (gnosis-agent-review--question-input state))
       (let ((attempts (plist-get (gnosis-agent-review--data state) :attempts)))
         (should (= calls 2))
         (should (= cancels 1))
         (should (= (length attempts) 2))
         (should (equal (plist-get (aref attempts 0) :error) "Cancelled"))
         (should-not (plist-get (aref attempts 0) :evaluation))
         (should (equal (plist-get (plist-get (aref attempts 1) :evaluation) :explanation)
                        "Successor result")))))))

(ert-deftest gnosis-test-local-reopened-reader-cannot-cancel-old-request ()
  (gnosis-test-local--with-session
   (let ((gnosis-agent-review-function (lambda (&rest _) #'ignore)))
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda ()
                  (insert "Interrupted pending answer")
                  (gnosis-agent-eval-submit)
                  (signal 'quit nil))))
       (condition-case nil (gnosis-agent-review--question-input state) (quit nil))))
   (let ((before (gnosis-agent-review--copy (plist-get (gnosis-agent-review--data state) :attempts))))
     (gnosis-sqlite-close gnosis-db)
     (setq gnosis-db (gnosis-sqlite-open gnosis-test--db-file))
     (gnosis-db-init)
     (setq state (gnosis-review--read-session) gnosis-review--state state)
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda () (gnosis-agent-eval-cancel) (signal 'quit nil))))
       (condition-case nil (gnosis-agent-review--question-input state) (quit nil)))
     (should (equal before (plist-get (gnosis-agent-review--data (gnosis-review--read-session)) :attempts))))))

(defun gnosis-test-local--parsed-plan (question)
  "Round-trip QUESTION in a plan through the real provider JSON parser."
  (gnosis-agent-romacs--parse-review
   (decode-coding-string
    (json-serialize (list :agent-note "Next α" :context nil :remaining (vector question) :done :false)
                    :false-object :false :null-object nil) 'utf-8)))

(ert-deftest gnosis-test-local-plan-key-order-and-content-identity ()
  (let ((keys '(:id :question :reference-answer :rubric :hints :parathema :tags :source)))
    (dolist (first keys)
      (gnosis-test-local--with-session
       (setq state (gnosis-test-local--reserve
                    (vector (gnosis-test-local--question "q1") (gnosis-test-local--question "q2")))
             gnosis-review--state state)
       (let ((gnosis-agent-review-function
              (lambda (_request resolve _reject)
                (funcall resolve '(:verdict "pass" :explanation "Correct.")) #'ignore)))
         (gnosis-test-local--answer state))
       (let* ((q (gnosis-test-local--question "q2"))
              (order (append (memq first keys) (seq-take keys (seq-position keys first))))
              (reordered (mapcan (lambda (key) (list key (plist-get q key))) order)))
         ;; Every authored field remains exact, including whitespace and vectors.
         (dolist (key (cdr keys))
           (let ((changed (gnosis-agent-review--copy reordered)))
             (setf (plist-get changed key)
                   (if (memq key '(:hints :tags)) ["Changed"]
                     (concat (plist-get q key) " ")))
             (should-error (gnosis-agent-review--plan (gnosis-agent-review--data state)
                                                      (gnosis-test-local--parsed-plan changed))
                           :type 'user-error)
             (when (eq first :id)
               (let ((gnosis-agent-review-function
                      (lambda (_request resolve _reject)
                        (funcall resolve (gnosis-test-local--parsed-plan changed)) #'ignore))
                     refused)
                 (cl-letf (((symbol-function 'read-multiple-choice)
                            (lambda (prompt _choices)
                              (setq refused prompt) '(?p "pause"))))
                   (catch 'review-loop (gnosis-agent-review--adapt state t)))
                 (should (equal refused "Changed question requires a new ID"))
                 (should (equal "adapt" (plist-get (gnosis-agent-review--data state) :phase)))
                 (should (equal '("q2") (gnosis-review-state-remaining state)))))))
         (should-error (gnosis-agent-review--plan
                        (gnosis-agent-review--data state)
                        (gnosis-test-local--parsed-plan (append reordered '(:unknown "No"))))
                       :type 'user-error)
         (should-error (gnosis-agent-romacs--parse-review
                        "{\"remaining\":[{\"id\":\"q2\",\"id\":\"q2\"}]}"))
         (let ((gnosis-agent-review-function
                (lambda (_request resolve _reject)
                  (funcall resolve (gnosis-test-local--parsed-plan reordered)) #'ignore)))
           (cl-letf (((symbol-function 'read-multiple-choice)
                      (lambda (&rest _) (ert-fail "Key order must not require recovery"))))
             (gnosis-agent-review--adapt state t)))
         (should (equal (gnosis-review-state-remaining state) '("q2")))
         (should (equal q (gnosis-agent-review--find (gnosis-agent-review--data state) "q2")))
         (gnosis-agent-review--present state)
         (should (equal "q2" (plist-get (plist-get (gnosis-agent-review--data state) :current) :id))))))))

(ert-deftest gnosis-test-local-romacs-provider-no-fallback ()
  (let ((original-require (symbol-function 'require))
        (gnosis-agent-romacs-provider "chosen-tutor")
        (gnosis-agent-romacs-model "chosen-model")
        calls resolutions rejections)
    (cl-letf (((symbol-function 'require)
               (lambda (feature &rest args)
                 (if (eq feature 'romacs-agent) t
                   (apply original-require feature args))))
              ((symbol-function 'romacs-agent-ask)
               (lambda (_prompt resolve _reject &rest options)
                 (push options calls)
                 (funcall resolve "{\"verdict\":\"pass\",\"explanation\":\"Correct\"}")
                 #'ignore)))
      (dolist (phase '("evaluate" "adapt"))
        (gnosis-agent-review-romacs (list :phase phase)
                                   (lambda (value) (push value resolutions))
                                   (lambda (failure) (push failure rejections)))))
    (should (= 2 (length calls)))
    (dolist (options calls)
      (should (equal (plist-get options :provider) "chosen-tutor"))
      (should (equal (plist-get options :model) "chosen-model")))
    (should (= 2 (length resolutions)))
    (should-not rejections)
    (setq calls nil resolutions nil)
    (let ((gnosis-agent-romacs-provider "missing-tutor"))
      (cl-letf (((symbol-function 'require)
                 (lambda (feature &rest args)
                   (if (eq feature 'romacs-agent) t
                     (apply original-require feature args))))
                ((symbol-function 'romacs-agent-ask)
                 (lambda (_prompt resolve reject &rest options)
                   (push (plist-get options :provider) calls)
                   (funcall reject "Unknown provider")
                   (funcall reject "Duplicate rejection")
                   (funcall resolve "{\"verdict\":\"pass\",\"explanation\":\"Late\"}")
                   #'ignore)))
        (gnosis-agent-review-romacs '(:phase "adapt")
                                   (lambda (value) (push value resolutions))
                                   (lambda (failure) (push failure rejections)))))
    (should (equal calls '("missing-tutor")))
    (should (equal rejections '("Unknown provider")))
    (should-not resolutions)))

(ert-deftest gnosis-test-local-ui-prose-response-and-viewport ()
  "Keep all verdict prose navigable without touching the response or view."
  (dolist (verdict '(pass fail ungradable error))
    (gnosis-test-local--with-session
     (save-window-excursion
       (switch-to-buffer review)
       (let* ((prose (concat (make-string 4000 ?x) "\n\nFinal paragraph α 🧠."))
              (response "Literal response α\nsecond line 50%")
              resolve reject submitted
              (gnosis-agent-review-function
               (lambda (request yes no)
                 (setq resolve yes reject no submitted (plist-get request :response))
                 #'ignore)))
         (cl-letf (((symbol-function 'recursive-edit)
                    (lambda ()
                      (buffer-enable-undo)
                      (insert response)
                      (undo-boundary)
                      (goto-char 5)
                      (let* ((draft (current-buffer))
                             (undo (copy-tree buffer-undo-list))
                             (windows (mapcar (lambda (w) (list w (window-point w) (window-start w)))
                                              (window-list)))
                             (position (point)))
                        (gnosis-agent-eval-submit)
                        (with-temp-buffer
                          (if (eq verdict 'error) (funcall reject prose)
                            (funcall resolve (list :verdict (symbol-name verdict) :explanation prose))))
                        (should (eq (current-buffer) draft))
                        (should (= position (point)))
                        (should (equal response submitted))
                        (should (equal response (buffer-string)))
                        (should (equal undo buffer-undo-list))
                        (dolist (entry windows)
                          (should (= (nth 1 entry) (window-point (car entry))))
                          (should (= (nth 2 entry) (window-start (car entry)))))
                        (should word-wrap)
                        (should-not truncate-partial-width-windows)
                        (should-not (string-match-p "Final paragraph"
                                     (overlay-get (plist-get gnosis-agent-eval--context :overlay) 'after-string)))
                        (with-current-buffer review
                          (save-excursion
                            (goto-char (point-max))
                            (should (string-match-p "Final paragraph α 🧠." (buffer-string)))
                            (search-backward "Final paragraph α 🧠." nil t)
                            (let ((inhibit-read-only nil))
                              (should-error (delete-char 1) :type 'text-read-only))
                            (should (equal (buffer-substring-no-properties (point) (+ (point) (length "Final paragraph α 🧠.")))
                                           "Final paragraph α 🧠."))))
                        (gnosis-agent-eval-cancel)
                        (should-not buffer-read-only)
                        (insert "z")
                        (undo-boundary)
                        (let ((last-command 'nil)) (undo 1))
                        (should (equal response (buffer-string)))
                        (with-current-buffer review
                          (should-not (string-match-p "Final paragraph" (buffer-string)))))
                      (signal 'quit nil))))
           (condition-case nil (gnosis-agent-review--question-input state) (quit nil))))))))

(ert-deftest gnosis-test-local-teaching-feedback-precedes-reference ()
  "Teach at the native feedback entry, not beneath the reference and answer."
  (dolist (response '("I don't know, please explain" "It closes."
                      "It opens." "It opens; flow increases."))
    (gnosis-test-local--with-session
     (let ((gnosis-agent-review-function
            (lambda (_request resolve _reject)
              (funcall resolve '(:verdict "fail" :explanation "Opening increases flow."))
              #'ignore)))
       (cl-letf (((symbol-function 'recursive-edit)
                  (lambda () (insert response) (gnosis-agent-eval-submit)))
                 ((symbol-function 'gnosis-review--read-action)
                  (lambda (_context)
                    (let ((text (buffer-string)))
                      (should (< (string-match "Evaluator feedback:" text)
                                 (string-match "Reference answer:" text)))
                      (should (string-match-p "Recall: not yet correct" text)))
                    ?n)))
         (gnosis-agent-review--question-input state))))))

(ert-deftest gnosis-test-local-native-one-submit-teaching ()
  "One submit reaches readable native feedback without accepting evidence."
  (skip-unless (not noninteractive))
  (dolist (example '(("I don't know, please explain\nα 🧠" "fail")
                     ("It closes; flow decreases." "fail")
                     ("It opens." "fail")
                     ("It opens; flow increases." "pass")))
    (gnosis-test-local--with-session
     (let* ((stage 0) failure driver
            (response (car example))
            (verdict (cadr example))
            (gnosis-agent-review-function
             (lambda (_request resolve _reject)
               (let ((value (list :verdict verdict :explanation "Opening increases flow.")))
                 (if (equal verdict "pass")
                     (progn (funcall resolve value) #'ignore)
                   (let ((timer (run-at-time 0.03 nil resolve value)))
                     (lambda () (cancel-timer timer))))))))
       (unwind-protect
           (progn
             (setq driver
                   (run-at-time
                    0.01 0.01
                    (lambda ()
                      (condition-case err
                          (when (and (= stage 0) (derived-mode-p 'gnosis-agent-eval-mode))
                            (insert response)
                            (setq stage 1 unread-command-events
                                  (listify-key-sequence (kbd "C-c C-c"))))
                        (error (setq failure err))))))
             (cl-letf (((symbol-function 'gnosis-review--read-action)
                        (lambda (_context)
                          (should (= stage 1))
                          (should (= 0 (gnosis-review-state-reviewed state)))
                          (should (eq (current-buffer) review))
                          (should (looking-at "Recall:"))
                          (should (string-match-p "Evaluator feedback:" (buffer-string)))
                          (setq stage 2)
                          ?n)))
               (with-timeout (2 (ert-fail "Second submit still required"))
                 (gnosis-agent-review--question-input state))))
         (cancel-timer driver))
       (when failure (signal (car failure) (cdr failure)))
       (should (= stage 2))
       (should (= 1 (gnosis-review-state-reviewed state)))
       (should (equal response (plist-get (aref (plist-get (gnosis-agent-review--data state) :encounters) 0)
                                         :response)))))))

(ert-deftest gnosis-test-local-native-ready-through-foreign-event-wait ()
  "Return feedback through a foreign event-preserving wait, not inside it."
  (skip-unless (not noninteractive))
  (dolist (same-buffer '(nil t))
    (gnosis-test-local--with-session
     (let* ((foreign (generate-new-buffer " *gnosis-test-event-wait*"))
            (response "Unknown α → β\nPlease explain 🧠")
            driver delivery resolver failure response-buffer undo
            wait-returned replayed draft-preserved actions
            (gnosis-agent-review-function
             (lambda (_request resolve _reject)
               (setq delivery
                     (run-at-time
                      0.01 nil
                      (lambda ()
			(condition-case err
                            (with-current-buffer (if same-buffer response-buffer foreign)
                              (setq resolver
                                    (run-at-time
                                     0.01 nil
                                     (lambda ()
                                       (funcall resolve '(:verdict "fail" :explanation "Opening increases flow."))
                                       (setq draft-preserved
                                             (with-current-buffer response-buffer
                                               (and (equal response (buffer-string))
                                                    (equal undo buffer-undo-list)))))))
                              ;; `dbus-call-method' preserves non-D-Bus events
                              ;; with this native reader.  Its current buffer
                              ;; need not be the selected response buffer.
                              (let ((event (let ((inhibit-redisplay t)
						 unread-command-events)
                                             (read-event nil nil 0.2))))
				(setq wait-returned t replayed event)
				(when event
                                  (setq unread-command-events
					(nconc unread-command-events (list event))))))
                          (error (setq failure err))))))
               #'ignore)))
       (unwind-protect
           (progn
             (setq driver
                   (run-at-time
                    0.01 0.01
                    (lambda ()
                      (when (and (not response-buffer)
				 (derived-mode-p 'gnosis-agent-eval-mode))
			(setq response-buffer (current-buffer))
			(buffer-enable-undo)
			(insert response)
			(undo-boundary)
			(setq undo (copy-tree buffer-undo-list)
                              unread-command-events
                              (append unread-command-events
                                      (listify-key-sequence (kbd "C-c C-c"))))))))
             (cl-letf (((symbol-function 'gnosis-review--read-action)
			(lambda (_context)
                          (should wait-returned)
                          (should (eq (car-safe replayed) 'gnosis-agent-eval-ready))
                          (should draft-preserved)
                          (should (zerop (gnosis-review-state-reviewed state)))
                          (should (eq (current-buffer) review))
                          (setq actions t)
                          ?n)))
               (with-timeout (2 (ert-fail "Ready event lost inside foreign native wait"))
		 (gnosis-agent-review--question-input state))))
	 (dolist (timer (list driver delivery resolver))
           (when (timerp timer) (cancel-timer timer)))
	 (kill-buffer foreign))
       (when failure (signal (car failure) (cdr failure)))
       (should actions)
       (should (equal response
                      (plist-get (aref (plist-get (gnosis-agent-review--data state) :encounters) 0)
				 :response)))))))

(ert-deftest gnosis-test-local-native-retry-feedback-readable ()
  "Expose retry feedback immediately; copy its end and retain exact draft undo."
  (skip-unless (not noninteractive))
  (dolist (failure-kind '(ungradable error))
    (gnosis-test-local--with-session
     (switch-to-buffer review)
     (let* ((stage 0) (calls 0) driver failure undo
            (response "Uncertain α\nPlease explain 🧠")
            (explanation (concat (make-string 4000 ?x) "\n\nFinal correction α 🧠."))
            (gnosis-agent-review-function
             (lambda (_request resolve reject)
               (cl-incf calls)
               (let ((timer
                      (run-at-time
                       0.03 nil
                       (lambda ()
                         (if (= calls 1)
                             (if (eq failure-kind 'error) (funcall reject explanation)
                               (funcall resolve (list :verdict "ungradable" :explanation explanation)))
                           (funcall resolve '(:verdict "pass" :explanation "Essentials established.")))))))
                 (lambda () (cancel-timer timer))))))
       (unwind-protect
           (progn
             (setq driver
                   (run-at-time
                    0.01 0.01
                    (lambda ()
                      (condition-case err
                          (when (derived-mode-p 'gnosis-agent-eval-mode)
                            (pcase stage
                              (0 (with-current-buffer review
                                   (let ((inhibit-read-only t))
                                     (goto-char (point-max))
                                     (insert (make-string 100 ?\n))))
                                 (buffer-enable-undo)
                                 (insert response) (undo-boundary)
                                 (setq undo (copy-tree buffer-undo-list)
                                       stage 1 unread-command-events
                                       (listify-key-sequence (kbd "C-c C-c"))))
                              (1
                               (when-let* ((section (plist-get gnosis-agent-eval--context :section))
                                           (window (get-buffer-window review)))
                                 (when (pos-visible-in-window-p (1+ (overlay-start section)) window)
                                   (should (equal (buffer-string) response))
                                   (should (equal buffer-undo-list undo))
                                   (should-not buffer-read-only)
                                   (should (= 0 (gnosis-review-state-reviewed state)))
                                   (with-selected-window window
                                     (execute-kbd-macro (kbd "M-> C-p C-a C-SPC C-e M-w"))
                                     (should (equal (car kill-ring) "Final correction α 🧠.")))
                                   (setq stage 2 unread-command-events
                                         (listify-key-sequence (kbd "C-c C-c"))))))))
                        (error (setq failure err unread-command-events
                                     (listify-key-sequence (kbd "C-g"))))))))
             (cl-letf (((symbol-function 'gnosis-review--read-action)
                        (lambda (_context) (should (= stage 2)) ?n)))
               (with-timeout (4 (ert-fail "Retry explanation was not exposed"))
                 (gnosis-agent-review--question-input state))))
         (cancel-timer driver))
       (when failure (signal (car failure) (cdr failure)))
       (should (= calls 2))
       (should (= 1 (gnosis-review-state-reviewed state)))))))

(ert-deftest gnosis-test-local-ready-event-owner-and-depth ()
  "A queued result never unwinds another input or supersedes cancellation."
  (gnosis-test-local--with-session
   (let ((gnosis-agent-review-function
          (lambda (_request resolve _reject)
            (funcall resolve '(:verdict "fail" :explanation "Opening increases flow.")) #'ignore)))
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda ()
                  (insert "Unknown")
                  (gnosis-agent-eval-submit)
                  (let* ((context gnosis-agent-eval--context)
                         (event (car (last unread-command-events)))
                         (depth (1+ (plist-get context :depth)))
                         exited)
                    (cl-letf (((symbol-function 'exit-recursive-edit) (lambda () (setq exited t))))
                      ;; The callback itself did not exit, nor may wrong depth.
                      (gnosis-agent-eval--ready event)
                      (should-not exited)
                      (cl-letf (((symbol-function 'recursion-depth) (lambda () depth)))
                        (with-current-buffer review (gnosis-agent-eval--ready event))
                        (should-not exited)
                        (gnosis-agent-eval-cancel)
                        (gnosis-agent-eval--ready event)
                        (should-not exited)))
                    (signal 'quit nil)))))
       (condition-case nil (gnosis-agent-review--question-input state) (quit nil)))
     (should-not (seq-some (lambda (event) (eq (car-safe event) 'gnosis-agent-eval-ready))
                          unread-command-events)))))

(ert-deftest gnosis-test-local-native-settled-reopen-needs-no-resubmit ()
  "Restore a settled response directly to teaching, without provider replay."
  (skip-unless (not noninteractive))
  (gnosis-test-local--with-session
   (let ((calls 0)
         (gnosis-agent-review-function
          (lambda (_request resolve _reject)
            (funcall resolve '(:verdict "fail" :explanation "Opening increases flow.")) #'ignore)))
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda ()
                  (insert "I don't know, please explain")
                  (gnosis-agent-eval-submit)
                  (signal 'quit nil))))
       (condition-case nil (gnosis-agent-review--question-input state) (quit nil)))
     (should (= 0 (gnosis-review-state-reviewed state)))
     (gnosis-sqlite-close gnosis-db)
     (setq gnosis-db (gnosis-sqlite-open gnosis-test--db-file))
     (gnosis-db-init)
     (setq state (gnosis-review--read-session) gnosis-review--state state)
     (let ((gnosis-agent-review-function (lambda (&rest _) (cl-incf calls) #'ignore))
           (choices 0))
       (cl-letf (((symbol-function 'gnosis-review--read-action)
                  (lambda (_context)
                    (should (looking-at "Recall: not yet correct"))
                    (cl-incf choices)
                    (if (= choices 1) ?o ?n))))
         (with-timeout (2 (ert-fail "Settled reopen still needs resubmission"))
           (gnosis-agent-review--question-input state)))
       (should (= choices 2)))
     (should (zerop calls))
     (let ((encounter (aref (plist-get (gnosis-agent-review--data state) :encounters) 0)))
       (should (equal (plist-get encounter :outcome) "success"))
       (should (eq (plist-get encounter :override) t))
       (should (equal (plist-get (plist-get encounter :evaluation) :verdict) "fail"))))))

(ert-deftest gnosis-test-local-tutor-policy-keeps-learning-evidence-distinct ()
  "Send teaching and finite revisit policy through the existing prompt only."
  (let ((evaluation (gnosis-agent-romacs--review-prompt '(:phase "evaluate")))
        (adaptation (gnosis-agent-romacs--review-prompt '(:phase "adapt"))))
    (dolist (policy '("smallest useful model" "demonstrated part" "clarification question"
                      "contradictory wording" "clear spelling errors" "captured rubric"))
      (should (string-match-p policy evaluation)))
    (dolist (policy '("prerequisite" "application" "intervening" "uncued revisit"
                      "learning debt" "help stage" "finite" "unresolved" "untested"
                      "not mastery" "redirect"))
      (should (string-match-p policy adaptation)))))

(ert-deftest gnosis-test-local-ui-labels-and-summary ()
  (gnosis-test-local--with-session
   (let ((gnosis-agent-review-function
          (lambda (_request yes _no)
            (funcall yes '(:verdict "pass" :explanation "Evaluator prose")) #'ignore)))
     (gnosis-test-local--answer state)
     (dolist (label '("Reference answer:" "Evaluator feedback:" "Explanation:"))
       (should (string-match-p label (buffer-string))))
     (should (= 1 (how-many "Evaluator feedback:" (point-min) (point-max))))
     (save-window-excursion
       (gnosis-agent-review--summary state)
       (unwind-protect
           (progn
             (should (string-match-p "Paused — adjustment pending" (buffer-string)))
             (should (eq (key-binding "?") #'gnosis-agent-review-summary-mode-map-popup))
             (should (gnosis-agent-review--summary-unfinished-p))
             (should-not truncate-partial-width-windows)
             (should-not (eq (key-binding "u") #'gnosis-review-undo)))
         (kill-buffer (current-buffer)))))))

(ert-deftest gnosis-test-local-ui-adaptation-status-settlement ()
  (dolist (outcome '(success error quit timeout))
    (gnosis-test-local--with-session
     (let ((gnosis-agent-review-function
            (lambda (_request yes _no)
              (funcall yes '(:verdict "pass" :explanation "Correct")) #'ignore)))
       (gnosis-test-local--answer state))
     (let* ((original header-line-format)
            (gnosis-agent-eval-timeout 0)
            seen
            (gnosis-agent-review-function
             (lambda (_request yes no)
               (setq seen header-line-format)
               (pcase outcome
                 ('success (funcall yes '(:agent-note "Done" :context nil :remaining [] :done t)))
                 ('error (funcall no "Offline"))
                 ('quit (signal 'quit nil))
                 ('timeout nil))
               #'ignore)))
       (cl-letf (((symbol-function 'read-multiple-choice) (lambda (&rest _) '(?p "pause"))))
         (condition-case nil (catch 'review-loop (gnosis-agent-review--adapt state t)) (quit nil)))
       (should (stringp seen))
       (should (string-match-p "Adjusting next question" seen))
       (should (string-match-p "C-g" seen))
       (should (equal header-line-format original))))))

(ert-deftest gnosis-test-local-ui-override-preserves-both-windows ()
  (gnosis-test-local--with-session
   (save-window-excursion
     (switch-to-buffer review)
     (delete-other-windows)
     (let ((gnosis-agent-review-function
            (lambda (_request yes _no)
              (funcall yes (list :verdict "fail"
                                 :explanation (mapconcat #'number-to-string (number-sequence 1 100) "\n")))
              #'ignore))
           (choices 0))
       (cl-letf (((symbol-function 'recursive-edit)
                  (lambda () (insert "Response") (gnosis-agent-eval-submit)))
                 ((symbol-function 'gnosis-review--read-action)
                  (lambda (_context)
                    (cl-incf choices)
                    (if (= choices 1)
                        (progn
                          (let ((other (split-window-below)))
                            (goto-char (point-min)) (forward-line 12)
                            (set-window-start (selected-window) (point))
                            (forward-line 3)
                            (set-window-point other (point))
                            (set-window-start other (point-min)))
                          ?o)
                      ?q))))
         (let ((original (symbol-function 'gnosis-display-next-review)))
           (cl-letf (((symbol-function 'gnosis-display-next-review)
                      (lambda (interval success)
                        (if (not gnosis-review--status) (funcall original interval success)
                          (let ((point (point))
                                (views (mapcar (lambda (w) (list w (window-point w) (window-start w)))
                                               (get-buffer-window-list review))))
                            (funcall original interval success)
                            (should (= point (point)))
                            (dolist (view views)
                              (should (= (nth 1 view) (window-point (car view))))
                              (should (= (nth 2 view) (window-start (car view))))))))))
             (catch 'review-loop (gnosis-agent-review--question-input state)))))
       (should (= choices 2))))))

(ert-deftest gnosis-test-local-prose-protected-interior-and-boundaries ()
  "Protect native edits inside prose, not adjacent independently owned text."
  (gnosis-test-local--with-session
   (switch-to-buffer review)
   (let ((gnosis-agent-review-function
          (lambda (_request yes _no)
            (funcall yes '(:verdict "ungradable" :explanation "Protected α feedback")) #'ignore)))
     (cl-letf (((symbol-function 'recursive-edit)
                (lambda ()
                  (insert "Exact response")
                  (gnosis-agent-eval-submit)
                  (let ((context gnosis-agent-eval--context))
                    (with-selected-window (get-buffer-window review)
                      (let* ((section (plist-get context :section))
                             (start (overlay-start section))
                             (end (overlay-end section))
                             (before (buffer-string))
                             (kill-ring '("YANK"))
                             (kill-ring-yank-pointer kill-ring)
                             (inhibit-read-only nil))
                        (cl-loop for position from (1+ start) below end do
                                 (goto-char position)
                                 (should-error (insert "x") :type 'text-read-only)
                                 (should-error (call-interactively #'yank) :type 'text-read-only)
                                 (should-error (delete-char 1) :type 'text-read-only)
                                 (should-error (delete-char -1) :type 'text-read-only))
                        (unless noninteractive
                          (with-selected-window (get-buffer-window review)
                            (dolist (position (list (1+ start) (- end 2)))
                              (goto-char position)
                              (should-error (execute-kbd-macro (kbd "C-y"))
                                            :type 'text-read-only))))
                        (goto-char start)
                        (should-error (delete-char 1) :type 'text-read-only)
                        (goto-char end)
                        (should-error (delete-char -1) :type 'text-read-only)
                        (should (equal before (buffer-string)))
                        ;; Both outer insertion points belong to outside text.
                        (goto-char start) (kill-new "Before section")
                        (if noninteractive (call-interactively #'yank)
                          (execute-kbd-macro (kbd "C-y")))
                        (delete-char -1) (insert "n")
                        (should (= (overlay-start section) (+ start (length "Before section"))))
                        (goto-char (overlay-end section)) (kill-new "After section")
                        (if noninteractive (call-interactively #'yank)
                          (execute-kbd-macro (kbd "C-y")))
                        (backward-char) (delete-char 1) (insert "n")))
                    ;; Owned replacement/removal must not absorb either neighbor.
                    (gnosis-agent-eval--prose context "Replacement feedback")
                    (gnosis-agent-eval-cancel)
                    (with-current-buffer review
                      (should-not (string-match-p "feedback" (buffer-string)))
                      (should (string-suffix-p "Before sectionAfter section" (buffer-string)))))
                  (signal 'quit nil))))
       (condition-case nil (gnosis-agent-review--question-input state) (quit nil))))))

(ert-deftest gnosis-test-local-prose-retired-response-cleanup-and-resume ()
  "Remove only owned prose after response retirement, retaining its exact draft."
  (dolist (retirement '(mode kill file))
    (gnosis-test-local--with-session
     (let* ((response "Exact draft α\nsecond line") draft context late entered
            (gnosis-agent-review-function
             (lambda (_request yes _no)
               (setq late yes)
               (funcall yes '(:verdict "pass" :explanation "Retired prose")) #'ignore)))
       (unwind-protect
           (progn
             (cl-letf (((symbol-function 'recursive-edit)
                        (lambda ()
                          (setq draft (current-buffer) context gnosis-agent-eval--context)
                          (insert response)
                          (gnosis-agent-eval-submit)
                          (pcase retirement
                            ('mode (text-mode))
                            ('kill (kill-buffer (current-buffer)))
                            ('file (set-visited-file-name (expand-file-name "retired-draft" gnosis-dir) t))))))
               (should-error (gnosis-agent-review--question-input state) :type 'user-error))
             (should-not (gnosis-agent-eval--valid-p context))
             (should-not (string-match-p "Retired prose" (buffer-string)))
             (should (equal response
                            (plist-get (plist-get (gnosis-agent-review--data (gnosis-review--read-session))
                                                 :current) :response)))
             (let ((before (buffer-string)) (checkpoint (gnosis-review--session-target)))
               (funcall late '(:verdict "fail" :explanation "Late stale prose"))
               (should (equal before (buffer-string)))
               (should (equal checkpoint (gnosis-review--session-target))))
             (cl-letf (((symbol-function 'recursive-edit)
                        (lambda ()
                          (setq entered (buffer-substring-no-properties (point-min) (point-max)))
                          (signal 'quit nil))))
               (condition-case nil (gnosis-review-resume) (quit nil)))
             (should (equal entered response)))
         (when (buffer-live-p draft)
           (with-current-buffer draft
             (set-buffer-modified-p nil)
             (kill-buffer draft))))))))

(ert-deftest gnosis-test-local-prose-cleanup-preserves-replaced-review ()
  "Do not reclaim an old section after native review setup or mode replacement."
  (dolist (replacement '(setup mode))
    (gnosis-test-local--with-session
     (let* (context late successor
           (gnosis-agent-review-function
            (lambda (_request yes _no)
              (setq late yes)
              (funcall yes '(:verdict "pass" :explanation "Old section")) #'ignore)))
       (cl-letf (((symbol-function 'recursive-edit)
                  (lambda ()
                    (insert "Exact response")
                    (gnosis-agent-eval-submit)
                    (setq context gnosis-agent-eval--context)
                    (with-current-buffer review
                      (pcase replacement
                        ('setup (gnosis-review--setup-buffer nil 'practice))
                        ('mode (text-mode)))
                      ;; Retain the old section's live range: cleanup must not
                      ;; delete it merely because its overlay still exists.
                      (goto-char (point-min)) (insert "Successor prefix\n")
                      (setq successor (buffer-string)))
                    (funcall late '(:verdict "fail" :explanation "Stale callback")))))
         (should-error (gnosis-agent-review--question-input state) :type 'user-error))
       (with-current-buffer review
         (should (equal-including-properties successor (buffer-string)))
         (gnosis-agent-eval--prose context nil)
         (should (equal-including-properties successor (buffer-string))))))))

(ert-deftest gnosis-test-local-native-prose-retirement-resume ()
  "Exercise mode, kill and file retirement from the native command loop."
  (skip-unless (not noninteractive))
  (dolist (retirement '(mode kill file))
    (gnosis-test-local--with-session
     (let* ((stage 0) timer failure draft entered
            (response "Native exact draft α\nsecond line")
            (gnosis-agent-review-function
             (lambda (_request yes _no)
               ;; Retirement already requested by the user takes precedence
               ;; over the queued transition, even after synchronous settlement.
               (setq stage 2 unread-command-events
                     (listify-key-sequence
                      (kbd (pcase retirement
                             ('mode "M-x text-mode RET")
                             ('kill "M-x kill-current-buffer RET")
                             ('file (concat "M-x set-visited-file-name RET "
                                            (expand-file-name "native-retired-draft" gnosis-dir)
                                            " RET"))))))
               (funcall yes '(:verdict "pass" :explanation "Native retired prose")) #'ignore)))
       (setq timer
             (run-at-time
              0.03 0.03
              (lambda ()
                (when (and (eq major-mode 'gnosis-agent-eval-mode) (> (recursion-depth) 0))
                  (condition-case err
                      (when (= stage 0)
                        (setq draft (current-buffer)) (insert response)
                        (setq stage 1 unread-command-events (listify-key-sequence (kbd "C-c C-c"))))
                    ((error quit) (setq failure err unread-command-events (listify-key-sequence (kbd "C-g")))))))))
       (unwind-protect
           (progn
             (unwind-protect
                 (condition-case nil
                     (with-timeout (10 (ert-fail "Native retirement timed out"))
                       (gnosis-agent-review--question-input state))
                   (quit nil))
               (cancel-timer timer))
             (when failure (signal (car failure) (cdr failure)))
             (should (= stage 2))
             (should-not (string-match-p "Native retired prose" (buffer-string)))
             (should (equal response
                            (plist-get (plist-get (gnosis-agent-review--data (gnosis-review--read-session))
                                                 :current) :response)))
             (cl-letf (((symbol-function 'recursive-edit)
                        (lambda ()
                          (setq entered (buffer-substring-no-properties (point-min) (point-max)))
                          (signal 'quit nil))))
               (condition-case nil (gnosis-review-resume) (quit nil)))
             (should (equal response entered)))
         (cancel-timer timer)
         (when (buffer-live-p draft)
           (with-current-buffer draft (set-buffer-modified-p nil) (kill-buffer draft))))))))

(provide 'gnosis-test-agent-review)
;;; gnosis-test-agent-review.el ends here
