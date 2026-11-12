;;; gnosis-test-agent-history.el --- Cold evidence discovery -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Free Software Foundation, Inc.

(require 'ert)
(require 'gnosis-agent-review)
(require 'gnosis-test-helpers)

(defun gnosis-test-history--reserve (&optional id)
  "Reserve a synthetic tutor question with ID, without native launch."
  (cl-letf (((symbol-function 'gnosis-agent--schedule) #'ignore))
    (gnosis-agent-start-review-session
     :questions (vector (list :id (or id "valve") :type "basic"
                              :question "Which way does the fictional valve move?"
                              :answer ["opens"] :choices [] :rubric nil :hints []
                              :parathema "Opening increases fictional flow."
                              :tags ["synthetic"] :source "Synthetic valve source"))
     :goal "Synthetic valve recall" :source "Synthetic only")))

(defun gnosis-test-history--collection (id mode)
  "Retain a minimal collection session ID in MODE using the native writer."
  (gnosis-review--save-history
   (gnosis-review-state-create :session-id id :mode mode :remaining '(101)
                               :selected '(101) :initial 1 :total 1)))

(ert-deftest gnosis-history-cold-discover-exact-evidence ()
  (gnosis-test-with-db
    (let* ((gnosis-review-buffer-name (generate-new-buffer-name "*history test*"))
           (gnosis-agent-review--providers nil)
           (gnosis-agent-review--background nil)
           (batch (gnosis-test-history--reserve))
           (state (gnosis-review--read-session))
           (review (gnosis-review--setup-buffer nil 'practice)))
      (unwind-protect
          (progn
            (with-current-buffer review
              (setq gnosis-review--state state)
              (cl-letf (((symbol-function 'gnosis--read-string-with-input-method)
                         (lambda (&rest _) "closes"))
                        ((symbol-function 'gnosis-review--read-action) (lambda (_) ?n)))
                (gnosis-agent-review--run state)))
            ;; Replace the completed checkpoint, then reopen with no saved ID
            ;; supplied to discovery.  The new session is not the evidence.
            (gnosis-test-history--reserve "successor")
            (gnosis-sqlite-close gnosis-db)
            (setq gnosis-db (gnosis-sqlite-open gnosis-test--db-file))
            (let* ((before (sqlite-select gnosis-db "SELECT total_changes()"))
                   (checkpoint (gnosis-get 'data 'study-session '(= id 1)))
                   (page (gnosis-agent-sessions :connection gnosis-db :kind "agent-review"))
                   (completed (seq-find (lambda (item) (equal (plist-get item :status) "completed"))
                                        (plist-get page :items)))
                   (result (gnosis-agent-results (plist-get completed :session-id)))
                   (encounter (aref (plist-get (plist-get result :study) :encounters) 0)))
              (should (equal (plist-get completed :session-id) (plist-get batch :session-id)))
              (should (equal (plist-get page :database) gnosis-test--db-file))
              (should (equal (plist-get encounter :outcome) "failure"))
              (should (equal (plist-get encounter :response) "closes"))
              (should (equal (plist-get (plist-get encounter :question) :answer) ["opens"]))
              (should (equal before (sqlite-select gnosis-db "SELECT total_changes()")))
              (should (equal checkpoint (gnosis-get 'data 'study-session '(= id 1))))
              (should (stringp (json-serialize page :false-object :false :null-object nil)))))
        (gnosis-agent-review--stop-background)
        (when (buffer-live-p review) (kill-buffer review))))))

(ert-deftest gnosis-history-bounded-mixed-empty-page-and-reopen ()
  (gnosis-test-with-db
    (let ((old (plist-get (gnosis-test-history--reserve) :session-id)))
      (gnosis-test-history--reserve "replacement")
      (gnosis-test-history--collection "due" 'due)
      (gnosis-test-history--collection "practice" 'practice)
      (let* ((before (sqlite-select gnosis-db "SELECT total_changes()"))
             (page (gnosis-agent-sessions :connection gnosis-db :kind "agent-review" :limit 2))
             (cursor (plist-get page :next-cursor))
             (next (gnosis-agent-sessions :connection gnosis-db :kind "agent-review" :limit 2 :cursor cursor)))
        (should (equal (plist-get page :items) []))
        (should (= (plist-get page :examined) 2))
        (should (eq (plist-get page :complete) :false))
        (should cursor)
        (should (= (length (plist-get next :items)) 2))
        (should (eq (plist-get next :complete) t))
        (should-not (plist-get next :next-cursor))
        (should (equal (plist-get (aref (plist-get next :items) 1) :session-id) old))
        (should (equal (plist-get (aref (plist-get next :items) 1) :status) "cancelled"))
        (should (equal before (sqlite-select gnosis-db "SELECT total_changes()")))
        (should-error (gnosis-agent-sessions :connection gnosis-db :cursor cursor) :type 'user-error)
        (gnosis-sqlite-close gnosis-db)
        (setq gnosis-db (gnosis-sqlite-open gnosis-test--db-file))
        (should-error (gnosis-agent-sessions :connection gnosis-db :kind "agent-review" :cursor cursor)
                      :type 'user-error)
        (should (= 4 (length (plist-get (gnosis-agent-sessions :connection gnosis-db) :items))))))))

(ert-deftest gnosis-history-malformed-unsupported-and-oversized-advance ()
  (gnosis-test-with-db
    (gnosis-test-history--reserve)
    (gnosis-sqlite-execute gnosis-db "INSERT INTO study_history VALUES (?, ?)"
                           '("future" (:version 99 :session-id "future" :mode practice)))
    (gnosis-sqlite-execute gnosis-db "INSERT INTO study_history VALUES (?, ?)" '("broken" "("))
    (sqlite-execute gnosis-db "INSERT INTO study_history VALUES (?, ?)"
                    (list "\"large\"" (make-string 2097153 ?x)))
    (let* ((page (gnosis-agent-sessions :connection gnosis-db :kind "agent-review" :limit 3))
           (next (gnosis-agent-sessions :connection gnosis-db :kind "agent-review" :limit 3
                                         :cursor (plist-get page :next-cursor))))
      (should (= (plist-get page :examined) 3))
      (should (equal (plist-get page :items) []))
      (should (equal (mapcar (lambda (issue) (plist-get issue :problem))
                             (append (plist-get page :issues) nil))
                     '("oversized" "malformed" "unsupported")))
      (should (= 1 (length (plist-get next :items))))
      (should (eq (plist-get next :complete) t)))))

(ert-deftest gnosis-history-insertion-order-not-id-or-update-order ()
  (gnosis-test-with-db
    ;; There are no session timestamps.  Arbitrary equal-time IDs must not
    ;; determine order; an update retains its original insertion position.
    (gnosis-test-history--collection "z" 'due)
    (gnosis-test-history--collection "a" 'practice)
    (gnosis-test-history--collection "m" 'due)
    (gnosis-test-history--collection "z" 'practice)
    (let (cursor ids)
      (dotimes (_ 3)
        (let ((page (gnosis-agent-sessions :connection gnosis-db :limit 1 :cursor cursor)))
          (setq ids (append ids (mapcar (lambda (item) (plist-get item :session-id))
                                        (append (plist-get page :items) nil)))
                cursor (plist-get page :next-cursor))
          (should (= (plist-get page :examined) 1))
          (should (equal (plist-get page :order) "history-insertion-desc"))))
      (should (equal ids '("m" "a" "z")))
      (should-not cursor))))

(ert-deftest gnosis-history-arguments-and-cursor-ownership ()
  (gnosis-test-with-db
    (should-error (gnosis-agent-sessions) :type 'user-error)
    (dolist (limit '(0 -1 51 1.5))
      (should-error (gnosis-agent-sessions :connection gnosis-db :limit limit) :type 'user-error))
    (should-error (gnosis-agent-sessions :connection gnosis-db :kind "mastered") :type 'user-error)
    (gnosis-test-history--collection "first" 'due)
    (gnosis-test-history--collection "second" 'practice)
    (let ((cursor (plist-get (gnosis-agent-sessions :connection gnosis-db :limit 1) :next-cursor)))
      (gnosis-test-with-db
        (should-error (gnosis-agent-sessions :connection gnosis-db :cursor cursor) :type 'user-error))
      (gnosis-test-history--collection "third" 'due)
      (should-error (gnosis-agent-sessions :connection gnosis-db :cursor cursor) :type 'user-error))))

(ert-deftest gnosis-history-legacy-v2-and-malformed-v1 ()
  (gnosis-test-with-db
    (gnosis-test-history--reserve)
    (let* ((state (gnosis-review--read-session))
           (data (gnosis-review-state-agent-review state)))
      (cl-remf data :protocol)
      (cl-remf data :proposal)
      (setf (gnosis-review-state-agent-review state) data)
      (gnosis-review--save-session state))
    (gnosis-sqlite-execute gnosis-db "INSERT INTO study_history VALUES (?, ?)"
                           '("incomplete" (:version 1 :session-id "incomplete" :mode practice)))
    (let ((page (gnosis-agent-sessions :connection gnosis-db)))
      (should (= 2 (plist-get page :examined)))
      (should (= 1 (length (plist-get page :issues))))
      (should (= 2 (plist-get (aref (plist-get page :items) 0) :version)))
      (should (equal "agent-review" (plist-get (aref (plist-get page :items) 0) :kind))))))

(ert-deftest gnosis-history-bounds-decoding-not-only-returned-matches ()
  (gnosis-test-with-db
    (gnosis-test-history--reserve)
    (dotimes (n 17) (gnosis-test-history--collection (number-to-string n) 'due))
    (let ((decode (symbol-function 'gnosis-agent--history-item)) (examined 0)
          (select (symbol-function 'sqlite-select)) (payloads 0))
      (cl-letf (((symbol-function 'gnosis-agent--history-item)
                 (lambda (row) (cl-incf examined) (funcall decode row)))
                ((symbol-function 'sqlite-select)
                 (lambda (connection query &rest args)
                   (let ((rows (apply select connection query args)))
                     (when (string-match-p "CASE WHEN length" query)
                       (cl-incf payloads (length rows)))
                     rows))))
        (let ((page (gnosis-agent-sessions :connection gnosis-db :limit 3 :kind "agent-review")))
          (should (equal [] (plist-get page :items)))
          (should (plist-get page :next-cursor))
          (should (= 3 examined payloads)))))))

(ert-deftest gnosis-history-external-and-schema-writes-expire-cursors ()
  (gnosis-test-with-db
    (gnosis-test-history--collection "first" 'due)
    (gnosis-test-history--collection "second" 'practice)
    (let ((cursor (plist-get (gnosis-agent-sessions :connection gnosis-db :limit 1) :next-cursor))
          (other (gnosis-sqlite-open gnosis-test--db-file)))
      (unwind-protect
          (sqlite-execute other "UPDATE study_history SET data = data || ' '")
        (gnosis-sqlite-close other))
      (should-error (gnosis-agent-sessions :connection gnosis-db :cursor cursor) :type 'user-error))
    (let ((cursor (plist-get (gnosis-agent-sessions :connection gnosis-db :limit 1) :next-cursor)))
      (sqlite-execute gnosis-db "VACUUM")
      (should-error (gnosis-agent-sessions :connection gnosis-db :cursor cursor) :type 'user-error))))

(ert-deftest gnosis-history-read-only-with-accepted-evidence ()
  (gnosis-test-with-db
    (gnosis-test--add-basic-thema "Synthetic" "answer" nil nil 101)
    (gnosis-scheduler-accept-review (gnosis-scheduler-event-id) 101 'success
                                    1780000000000000 (gnosis--today-int))
    (gnosis-study-accept-practice
     (list :mode 'practice :event-id "synthetic-practice" :thema-id 101
           :session-id "prior-practice" :attempt 1 :reviewed-at-us 1780000000000001
           :outcome 'failure))
    (gnosis-test-history--reserve)
    (let* ((tables '(themata scheduler-state scheduler-baseline review-events
                            practice-events study-session study-history))
           (before (mapcar (lambda (table) (gnosis-select '* table)) tables))
           (connection gnosis-db))
      (should (gnosis-select '* 'review-events))
      (should (gnosis-select '* 'practice-events))
      (sqlite-execute connection "PRAGMA query_only = ON")
      ;; A different ambient database must never become the discovery owner.
      (gnosis-test-with-db
        (should-not (eq connection gnosis-db))
        (let ((page (gnosis-agent-sessions :connection connection)))
          (should (= 1 (length (plist-get page :items))))))
      (should (equal before (mapcar (lambda (table) (gnosis-select '* table)) tables))))))

(ert-deftest gnosis-history-refuses-caller-transactions-without-settling-them ()
  (gnosis-test-with-db
    (gnosis-test-history--collection "older" 'practice)
    (gnosis-test-history--collection "newer" 'practice)
    (let ((cursor (plist-get (gnosis-agent-sessions :connection gnosis-db :limit 1)
                             :next-cursor))
          (other (sqlite-open gnosis-test--db-file)))
      (unwind-protect
          (progn
            (sqlite-execute gnosis-db "BEGIN")
            (let ((state (gnosis-review-state-create :session-id "newer" :mode 'practice)))
              (gnosis-review--save-history state))
            (let ((uncommitted (sqlite-select gnosis-db "SELECT * FROM study_history")))
              (should-error (gnosis-agent-sessions :connection gnosis-db) :type 'user-error)
              (should-error (gnosis-agent-sessions :connection gnosis-db :cursor cursor)
                            :type 'user-error)
              (should (equal uncommitted (sqlite-select gnosis-db "SELECT * FROM study_history")))
              (should-not (equal uncommitted (sqlite-select other "SELECT * FROM study_history"))))
            ;; The caller, not discovery, still owns the rollback.
            (sqlite-execute gnosis-db "ROLLBACK")
            (should-error (gnosis-agent-sessions :connection gnosis-db :cursor cursor)
                          :type 'user-error)
            (should (equal "unfinished"
                           (plist-get (aref (plist-get (gnosis-agent-sessions :connection gnosis-db)
                                                       :items) 0) :status)))
            ;; A caller's read transaction or outer savepoint is also untouched.
            (dolist (begin '("BEGIN" "SAVEPOINT caller"))
              (sqlite-execute gnosis-db begin)
              (should-error (gnosis-agent-sessions :connection gnosis-db) :type 'user-error)
              (sqlite-execute gnosis-db "COMMIT"))
            (should (= 2 (length (plist-get (gnosis-agent-sessions :connection gnosis-db) :items)))))
        (sqlite-close other)))))

(ert-deftest gnosis-history-malformed-keys-and-collection-queues-advance ()
  (gnosis-test-with-db
    (gnosis-test-history--collection "valid" 'practice)
    (let ((valid (gnosis-get 'data 'study-history '(= session-id "valid"))))
      (dolist (key '("\"valid\" trailing" " \"valid\"" "\"valid\" "))
        (sqlite-execute gnosis-db "INSERT INTO study_history VALUES (?, ?)"
                        (list key (gnosis-sqlite--serialize valid))))
      (dolist (queue '("#1#" "(101 . 102)" "(\"101\")" "(nil)" "(101.0)"))
        (let ((id (concat "bad-" queue)))
          (sqlite-execute gnosis-db "INSERT INTO study_history VALUES (?, ?)"
                          (list (gnosis-sqlite--serialize id)
                                (format "#1=(:version 1 :session-id %S :mode practice :cancelled-p nil :remaining %s)"
                                        id queue)))))
      (let ((page (gnosis-agent-sessions :connection gnosis-db :limit 8)))
        (should (equal [] (plist-get page :items)))
        (should (= 8 (length (plist-get page :issues))))
        (should (seq-every-p (lambda (issue) (equal "malformed" (plist-get issue :problem)))
                             (plist-get page :issues)))
        (let* ((next (gnosis-agent-sessions :connection gnosis-db :cursor (plist-get page :next-cursor)))
               (id (plist-get (aref (plist-get next :items) 0) :session-id)))
          (should (equal id "valid"))
          (should (eq t (plist-get next :complete)))
          (should (equal id (plist-get (gnosis-agent-results id) :session-id))))))))

(ert-deftest gnosis-history-rejects-cycles-but-preserves-shared-state ()
  (gnosis-test-with-db
    (let ((ids (list 101 102)))
      (gnosis-review--save-history
       (gnosis-review-state-create :session-id "shared" :mode 'practice
                                   :remaining ids :selected ids :initial 2 :total 2)))
    (gnosis-test-history--reserve)
    (let* ((state (gnosis-review--read-session))
           (local (gnosis-review-state-agent-review state)))
      (should (string-match-p "#[0-9]+=" (caar (sqlite-select gnosis-db "SELECT data FROM study_history ORDER BY rowid DESC LIMIT 1"))))
      (should (= 2 (length (plist-get (gnosis-agent-sessions :connection gnosis-db) :items))))
      (cl-remf local :protocol)
      (cl-remf local :proposal)
      (setf (gnosis-review-state-agent-review state) local)
      (gnosis-review--save-session state)
      (should (= 2 (plist-get (aref (plist-get (gnosis-agent-sessions :connection gnosis-db) :items) 0) :version))))
    ;; Cycles outside the remaining queue must not reach permissive decoders.
    (sqlite-execute gnosis-db "INSERT INTO study_history VALUES (?, ?)"
                    '("\"cycle\"" "(:version 1 :session-id \"cycle\" :mode practice :cancelled-p nil :remaining nil :selected #1=[#1#])"))
    (let ((page (gnosis-agent-sessions :connection gnosis-db :limit 1)))
      (should (equal [] (plist-get page :items)))
      (should (equal "malformed" (plist-get (aref (plist-get page :issues) 0) :problem))))))

(ert-deftest gnosis-history-read-snapshot-cleanup-and-wal-drift ()
  (gnosis-test-with-db
    (gnosis-test-history--collection "valid" 'practice)
    (sqlite-execute gnosis-db "PRAGMA journal_mode=WAL")
    (let ((other (sqlite-open gnosis-test--db-file))
          (decode (symbol-function 'gnosis-agent--history-item)))
      (unwind-protect
          (progn
            ;; Force a real external commit after acquisition of our snapshot.
            (cl-letf (((symbol-function 'gnosis-agent--history-item)
                       (lambda (row)
                         (sqlite-execute other "UPDATE study_history SET data=data || ' '")
                         (funcall decode row))))
              (should-error (gnosis-agent-sessions :connection gnosis-db) :type 'user-error))
            ;; Neither errors nor quits may leave our read transaction open.
            (dolist (condition '(error quit))
              (cl-letf (((symbol-function 'gnosis-agent--history-item)
                         (lambda (_) (signal condition nil))))
                (should (eq condition
                            (condition-case err
                                (gnosis-agent-sessions :connection gnosis-db)
                              ((error quit) (car err))))))
              (sqlite-execute gnosis-db "BEGIN")
              (sqlite-execute gnosis-db "ROLLBACK"))
            (let ((page (gnosis-agent-sessions :connection gnosis-db)))
              (should (= 1 (length (plist-get page :items))))
              (should (equal [] (plist-get page :issues)))))
        (sqlite-close other)))))

(provide 'gnosis-test-agent-history)
;;; gnosis-test-agent-history.el ends here
