;;; hermes-tracker-db-tests.el --- SQLite tests for Hermes Tracker -*- lexical-binding: t; -*-

;;; Code:

(require 'ert)
(require 'hermes-tracker-db)

(defmacro hermes-tracker-db-tests--with-database (&rest body)
  "Run BODY with an isolated Tracker database."
  (declare (indent 0) (debug t))
  `(let* ((directory (make-temp-file "hermes-tracker-db-" t))
          (hermes-tracker-db-file (expand-file-name "tracker.db" directory))
          (hermes-tracker-db--connection nil))
     (unwind-protect
         (progn
           (hermes-tracker-db-open)
           ,@body)
       (hermes-tracker-db-close)
       (delete-directory directory t))))

(ert-deftest hermes-tracker-db-persists-repositories-across-reopen ()
  "Repository rows survive closing and reopening the cache."
  (hermes-tracker-db-tests--with-database
    (hermes-tracker-db-upsert-repositories
     "origin"
     '(((slug . "emacs-hermes")
        (name . "Emacs Hermes")
        (description . "Frontend")))
     "2026-08-08T10:00:00Z")
    (hermes-tracker-db-close)
    (should
     (equal (hermes-tracker-db-list-repositories "origin")
            '(((slug . "emacs-hermes")
               (name . "Emacs Hermes")
               (description . "Frontend")))))
    (should (sqlitep hermes-tracker-db--connection))
    (should (= (logand (file-modes directory) #o777) #o700))
    (should (= (logand (file-modes hermes-tracker-db-file) #o777) #o600))))

(ert-deftest hermes-tracker-db-rejects-permissive-cache-directory ()
  "Opening the cache rejects rather than mutates an unsafe parent directory."
  (let* ((directory (make-temp-file "hermes-tracker-db-permissions-" t))
         (hermes-tracker-db-file (expand-file-name "tracker.db" directory))
         (hermes-tracker-db--connection nil))
    (unwind-protect
        (progn
          (set-file-modes directory #o777)
          (should-error (hermes-tracker-db-open) :type 'user-error)
          (should (= (logand (file-modes directory) #o777) #o777))
          (set-file-modes directory #o700)
          (write-region "" nil hermes-tracker-db-file nil 'silent)
          (set-file-modes hermes-tracker-db-file #o666)
          (hermes-tracker-db-open)
          (should (= (logand (file-modes directory) #o777) #o700))
          (should (= (logand (file-modes hermes-tracker-db-file) #o777)
                     #o600)))
      (hermes-tracker-db-close)
      (delete-directory directory t))))

(ert-deftest hermes-tracker-db-rejects-symbolic-link-cache ()
  "Opening the cache refuses a symbolic-link destination."
  (let* ((directory (make-temp-file "hermes-tracker-db-symlink-" t))
         (target (expand-file-name "target.db" directory))
         (hermes-tracker-db-file (expand-file-name "tracker.db" directory))
         (hermes-tracker-db--connection nil))
    (unwind-protect
        (progn
          (write-region "unchanged" nil target nil 'silent)
          (make-symbolic-link target hermes-tracker-db-file)
          (should-error (hermes-tracker-db-open) :type 'user-error)
          (should (equal (with-temp-buffer
                           (insert-file-contents target)
                           (buffer-string))
                         "unchanged")))
      (hermes-tracker-db-close)
      (delete-directory directory t))))

(ert-deftest hermes-tracker-db-accepts-transport-object-shapes ()
  "Boundary normalization accepts hash-table and plist API objects."
  (let ((todo (make-hash-table :test #'equal)))
    (dolist (pair '(("number" . 1) ("title" . "Hash") ("status" . "open")
                    ("priority" . 0) ("labels" . ["bug"])
                    ("assignee" . nil) ("author" . nil)
                    ("updated_at" . nil)
                    ("closed_at" . "stale")
                    ("verification_output" . "kept")))
      (puthash (car pair) (cdr pair) todo))
    (should (equal (hermes-tracker-db--todo-record todo)
                   '(1 "Hash" "open" 0 nil nil nil "stale" ("bug"))))
    (hermes-tracker-db-tests--with-database
      (hermes-tracker-db-upsert-repositories
       "origin" '(((slug . "project") (name . "Project"))) "now")
      (hermes-tracker-db-upsert-todo-detail
       "origin" "project" todo "now")
      (let ((cached (hermes-tracker-db-get-todo "origin" "project" 1)))
        (should (equal (alist-get 'verification_output cached) "kept"))
        (should-not (assq 'closed_at cached)))
      (should-not
       (caar (sqlite-select hermes-tracker-db--connection
                            "SELECT closed_at FROM todos WHERE number = 1")))))
  (should (equal (hermes-tracker-db--todo-record
                  '(:number 2 :title "Plist" :status "open"
                    :priority 0 :labels nil :assignee nil :author nil
                    :updated_at nil :closed_at nil))
                 '(2 "Plist" "open" 0 nil nil nil nil nil))))

(ert-deftest hermes-tracker-db-upsert-todos-is-atomic ()
  "Malformed TODO input leaves an existing repository snapshot unchanged."
  (hermes-tracker-db-tests--with-database
    (hermes-tracker-db-upsert-repositories
     "origin" '(((slug . "project") (name . "Project"))) "now")
    (hermes-tracker-db-upsert-todos
     "origin" "project"
     '(((number . 1) (title . "Existing") (status . "open")
        (priority . 0) (labels . ("bug")) (assignee . "alice")
        (author . "thanos") (updated_at . "now") (closed_at . nil))))
    (should-error
     (hermes-tracker-db-upsert-todos
      "origin" "project"
      '(((number . 2) (title . "Valid") (status . "open")
         (priority . 0) (labels . ("docs")))
        ((title . "Missing number") (status . "open")))))
    (should-error
     (hermes-tracker-db-upsert-todos
      "origin" "project"
      '(((number . 1) (title . "Invalid") (status . "bogus")
         (priority . 0) (labels . ("bad"))))))
    (should-error
     (hermes-tracker-db-upsert-todos
      "origin" "project"
      '(((number . 1) (title . "Missing labels") (status . "open")
         (priority . 0)))))
    (should-error
     (hermes-tracker-db-upsert-todos
      "origin" "project"
      '(((number . 1) (title . "Missing priority") (status . "open")
         (labels . ("bad"))))))
    (hermes-tracker-db-upsert-todos
     "origin" "project"
     '(((number . 1) (title . "Updated") (status . "open")
        (priority . 1) (labels . ("bug")))))
    (should
     (equal (hermes-tracker-db-query-todos "origin" "project" nil)
            '(((number . 1) (title . "Updated") (status . "open")
               (priority . 1) (assignee . "alice") (author . "thanos")
               (updated_at . "now") (closed_at . nil) (labels . ("bug"))))))))

(ert-deftest hermes-tracker-db-complete-snapshots-prune-absent-rows ()
  "Successful complete snapshots replace absent repositories and TODOs."
  (hermes-tracker-db-tests--with-database
    (hermes-tracker-db-upsert-repositories
     "origin"
     '(((slug . "kept") (name . "Kept"))
       ((slug . "removed") (name . "Removed")))
     "first")
    (hermes-tracker-db-upsert-todo-page
     "origin" '((slug . "kept") (name . "Kept"))
     '(((number . 1) (title . "Kept") (status . "open")
        (priority . 0) (labels . nil))
       ((number . 2) (title . "Removed") (status . "open")
        (priority . 0) (labels . ("stale"))))
     "first")
    (hermes-tracker-db-upsert-repositories
     "origin" '(((slug . "kept") (name . "Kept"))) "second")
    (should (equal (mapcar (lambda (repo) (alist-get 'slug repo))
                           (hermes-tracker-db-list-repositories "origin"))
                   '("kept")))
    (hermes-tracker-db-upsert-todo-page
     "origin" '((slug . "kept") (name . "Kept"))
     '(((number . 1) (title . "Updated") (status . "open")
        (priority . 0) (labels . nil)))
     "second")
    (should (equal (mapcar (lambda (todo) (alist-get 'number todo))
                           (hermes-tracker-db-query-todos
                            "origin" "kept" '(:state "all")))
                   '(1)))
    (hermes-tracker-db-upsert-todo-page
     "origin" '((slug . "kept") (name . "Kept")) nil "third")
    (should-not (hermes-tracker-db-query-todos
                 "origin" "kept" '(:state "all")))))

(ert-deftest hermes-tracker-db-queries-summary-fields-and-labels ()
  "Local filters preserve Tracker label-AND and assignee-OR semantics."
  (hermes-tracker-db-tests--with-database
    (hermes-tracker-db-upsert-repositories
     "origin" '(((slug . "project") (name . "Project"))) "now")
    (hermes-tracker-db-upsert-todos
     "origin" "project"
     '(((number . 1) (title . "Fix Auth Login") (status . "open")
        (priority . 5) (assignee . "alice") (author . "thanos")
        (labels . ("auth" "bug")) (updated_at . "2026-08-08T10:00:00Z"))
       ((number . 2) (title . "Auth documentation") (status . "closed")
        (priority . 0) (assignee . "bob") (labels . ("auth" "docs")))
       ((number . 3) (title . "Progress 100%") (status . "open")
        (priority . 0) (labels . nil))
       ((number . 4) (title . "Path a\\b_value") (status . "open")
        (priority . 0) (labels . nil))))
    (should
     (equal
      (mapcar (lambda (todo) (alist-get 'number todo))
              (hermes-tracker-db-query-todos
               "origin" "project"
               '(:state "all" :query "auth" :labels ("auth" "bug" "auth")
                 :assignees ("alice" "carol"))))
      '(1)))
    (should
     (equal (mapcar (lambda (todo) (alist-get 'number todo))
                    (hermes-tracker-db-query-todos "origin" "project" nil))
            '(1 3 4)))
    (should
     (equal (mapcar (lambda (todo) (alist-get 'number todo))
                    (hermes-tracker-db-query-todos
                     "origin" "project" '(:query "   ")))
            '(1 3 4)))
    (should-error
     (hermes-tracker-db-query-todos
      "origin" "project" '(:state "bogus")))
    (should
     (equal (mapcar (lambda (todo) (alist-get 'number todo))
                    (hermes-tracker-db-query-todos
                     "origin" "project" '(:state "all" :query "%")))
            '(3)))
    (dolist (query '("\\" "_"))
      (should
       (equal (mapcar (lambda (todo) (alist-get 'number todo))
                      (hermes-tracker-db-query-todos
                       "origin" "project" `(:state "all" :query ,query)))
              '(4))))
    (should
     (equal (alist-get 'status_counts
                       (alist-get 'summary
                                  (car (hermes-tracker-db-list-repository-summaries
                                        "origin"))))
            '((open . 3) (closed . 1))))
    (hermes-tracker-db-upsert-todos
     "origin" "project"
     '(((number . 1) (title . "Fix Auth Login") (status . "closed")
        (priority . 5) (assignee . "alice") (labels . ("security")))))
    (let ((todo (car (hermes-tracker-db-query-todos
                      "origin" "project" '(:state "closed")))))
      (should (equal (alist-get 'status todo) "closed"))
      (should (equal (alist-get 'labels todo) '("security"))))))

(ert-deftest hermes-tracker-db-summary-upsert-preserves-detail ()
  "Summary synchronization does not erase cached TODO detail."
  (hermes-tracker-db-tests--with-database
    (hermes-tracker-db-upsert-repositories
     "origin" '(((slug . "project") (name . "Project"))) "now")
    (hermes-tracker-db-upsert-todo-detail
     "origin" "project"
     '((number . 1) (title . "Initial") (status . "closed") (priority . 0)
       (description . "Detailed body") (labels . ("bug"))
       (closed_at . "2026-08-08T09:00:00Z") (closed_by . "talos")
       (closing_commit . "abc123") (verification_output . "Passed")
       (comments . (((id . 7) (body . "Comment")))))
     "now")
    (hermes-tracker-db-upsert-todos
     "origin" "project"
     '(((number . 1) (title . "Renamed") (status . "closed")
        (priority . 1) (labels . ("done")))))
    (let ((todo (hermes-tracker-db-get-todo "origin" "project" 1)))
      (should (equal (alist-get 'title todo) "Renamed"))
      (should (equal (alist-get 'status todo) "closed"))
      (should (equal (alist-get 'description todo) "Detailed body"))
      (should (equal (alist-get 'labels todo) '("done")))
      (should (= (alist-get 'id (car (alist-get 'comments todo))) 7)))
    (hermes-tracker-db-upsert-todos
     "origin" "project"
     '(((number . 1) (title . "Reopened") (status . "open")
        (priority . 1) (labels . ("follow-up")) (closed_at . nil))))
    (let ((todo (hermes-tracker-db-get-todo "origin" "project" 1)))
      (should (equal (alist-get 'title todo) "Reopened"))
      (should (equal (alist-get 'verification_output todo) "Passed"))
      (dolist (key '(closed_at closed_by closing_commit))
        (should-not (assq key todo))))
    (hermes-tracker-db-upsert-todos
     "origin" "project"
     '(((number . 1) (title . "Closed again") (status . "closed")
        (priority . 1) (labels . ("follow-up"))
        (closed_at . "2026-08-08T11:00:00Z"))))
    (let ((todo (hermes-tracker-db-get-todo "origin" "project" 1)))
      (should (equal (alist-get 'closed_at todo) "2026-08-08T11:00:00Z"))
      (should (equal (alist-get 'verification_output todo) "Passed"))
      (should-not (assq 'closed_by todo))
      (should-not (assq 'closing_commit todo)))))

(ert-deftest hermes-tracker-db-rejects-newer-schema ()
  "Opening a newer cache schema fails without deleting it."
  (hermes-tracker-db-tests--with-database
    (sqlite-execute hermes-tracker-db--connection
                    "UPDATE schema_meta SET version = 99")
    (hermes-tracker-db-close)
    (should-error (hermes-tracker-db-open)
                  :type 'hermes-tracker-db-schema-error)
    (should (file-exists-p hermes-tracker-db-file))))

(ert-deftest hermes-tracker-db-replaces-label-catalog-atomically ()
  "A catalog sync replaces labels for only its cache origin."
  (hermes-tracker-db-tests--with-database
    (hermes-tracker-db-replace-label-catalog
     "origin" '((labels . (((name . "bug") (color . "ff0000"))
                            ((name . "docs") (color . "00ff00"))))
                (fallback_color . "808080")) "now")
    (hermes-tracker-db-replace-label-catalog
     "other" '((labels . (((name . "private") (color . "000000"))))
               (fallback_color . "707070")) "now")
    (hermes-tracker-db-replace-label-catalog
     "origin" '((labels . (((name . "enhancement") (color . "0000ff"))))
                (fallback_color . "909090")) "later")
    (should-error
     (hermes-tracker-db-replace-label-catalog
      "origin" '((fallback_color . "ffffff")) "invalid"))
    (should (equal (hermes-tracker-db-list-labels "origin")
                   '(((name . "enhancement") (color . "0000ff")))))
    (should (equal (hermes-tracker-db-label-fallback-color "origin")
                   "909090"))
    (should (equal (mapcar (lambda (label) (alist-get 'name label))
                           (hermes-tracker-db-list-labels "other"))
                   '("private")))))

(provide 'hermes-tracker-db-tests)
;;; hermes-tracker-db-tests.el ends here
