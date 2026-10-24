;;; hermes-tracker-sync-tests.el --- Sync tests for Hermes Tracker -*- lexical-binding: t; -*-

;;; Code:

(require 'ert)
(require 'hermes-tracker-sync)

(defmacro hermes-tracker-sync-tests--with-state (&rest body)
  "Run BODY with isolated sync and database state."
  (declare (indent 0) (debug t))
  `(let* ((directory (make-temp-file "hermes-tracker-sync-" t))
          (hermes-tracker-db-file (expand-file-name "tracker.db" directory))
          (hermes-tracker-db--connection nil)
          (hermes-tracker-url "https://tracker.test")
          (hermes-tracker-auth-source-user "token-user")
          (hermes-tracker-auth-source-port "tracker")
          (hermes-tracker-actor "alice")
          (hermes-tracker-sync--in-flight (make-hash-table :test #'equal))
          (hermes-tracker-sync--epochs (make-hash-table :test #'equal))
          (hermes-tracker-sync-changed-hook nil)
          (hermes-tracker-sync--timer nil)
          (hermes-tracker-sync--mode-epoch 0)
          (hermes-tracker-sync-mode nil))
     (unwind-protect
         (progn ,@body)
       (when (timerp hermes-tracker-sync--timer)
         (cancel-timer hermes-tracker-sync--timer))
       (hermes-tracker-db-close)
       (delete-directory directory t))))

(defun hermes-tracker-sync-tests--repo (slug)
  "Return a minimal repository payload for SLUG."
  `((slug . ,slug) (name . ,(capitalize slug)) (description . nil)))

(defun hermes-tracker-sync-tests--todo (number title)
  "Return a minimal open TODO NUMBER named TITLE."
  `((number . ,number) (title . ,title) (status . "open")
    (priority . 0) (assignee . nil) (author . "thanos")
    (labels . nil) (updated_at . "2026-08-08T10:00:00Z")
    (closed_at . nil)))

(ert-deftest hermes-tracker-sync-origin-includes-actor-without-secrets ()
  "Cache identity is stable per endpoint/actor and excludes token material."
  (hermes-tracker-sync-tests--with-state
    (cl-letf (((symbol-function 'hermes-tracker--token)
               (lambda (&rest _) "secret-token")))
      (let* ((request (hermes-tracker--request "GET" "/api/v1/repos"))
             (alice (hermes-tracker-sync-cache-origin))
             (same (hermes-tracker-sync-cache-origin))
             (hermes-tracker-actor "bob")
             (bob (hermes-tracker-sync-cache-origin)))
        (should (equal (plist-get request :secrets) '("secret-token")))
        (should (equal alice same))
        (should-not (equal alice bob))
        (should-not (string-match-p "secret-token" alice))
        (hermes-tracker-db-record-sync-success alice "repos" "now")
        (should-not
         (string-match-p
          "secret-token"
          (caar (sqlite-select hermes-tracker-db--connection
                               "SELECT cache_origin FROM sync_state"))))
        (let* ((hermes-tracker-actor nil)
               (user-login-name "fallback-alice")
               (fallback-alice (hermes-tracker-sync-cache-origin))
               (user-login-name "fallback-bob")
               (fallback-bob (hermes-tracker-sync-cache-origin)))
          (should-not (equal fallback-alice fallback-bob)))
        (let* ((hermes-tracker-actor "  alice  ")
               (normalized (hermes-tracker-sync-cache-origin))
               (hermes-tracker-actor "alice")
               (plain (hermes-tracker-sync-cache-origin)))
          (should (equal normalized plain)))))))

(ert-deftest hermes-tracker-sync-coalesces-identical-resource-requests ()
  "One in-flight resource owns one HTTP request and one DB write."
  (hermes-tracker-sync-tests--with-state
    (let ((pending (hermes--promise-make))
          (requests 0)
          events)
      (add-hook 'hermes-tracker-sync-changed-hook
                (lambda (origin scope) (push (list origin scope) events)))
      (cl-letf (((symbol-function 'hermes-tracker--request-async)
                 (lambda (&rest _)
                   (cl-incf requests)
                   pending)))
        (let ((first (hermes-tracker-sync-todos "project"))
              (second (hermes-tracker-sync-todos "project")))
          (should (eq first second))
          (should (= requests 1))
          (hermes--promise-resolve
           pending
           `((repo . ,(hermes-tracker-sync-tests--repo "project"))
             (todos . (,(hermes-tracker-sync-tests--todo 1 "Cached")))))
          (should (equal (alist-get 'title
                                    (car (hermes-tracker-db-query-todos
                                          (hermes-tracker-sync-cache-origin)
                                          "project" '(:state "all"))))
                         "Cached"))
          (should (equal events
                         `((,(hermes-tracker-sync-cache-origin)
                            (todos "project"))))))))))

(ert-deftest hermes-tracker-sync-mutation-invalidates-older-response ()
  "A response started before a mutation cannot overwrite a newer sync."
  (hermes-tracker-sync-tests--with-state
    (let ((old (hermes--promise-make))
          (new (hermes--promise-make))
          (calls 0))
      (cl-letf (((symbol-function 'hermes-tracker--request-async)
                 (lambda (&rest _)
                   (cl-incf calls)
                   (if (= calls 1) old new))))
        (hermes-tracker-sync-todos "project")
        (hermes-tracker-sync-note-mutation "project")
        (hermes-tracker-sync-todos "project")
        (hermes--promise-resolve
         old `((repo . ,(hermes-tracker-sync-tests--repo "project"))
               (todos . (,(hermes-tracker-sync-tests--todo 1 "Old")))))
        (should-not (hermes-tracker-db-query-todos
                     (hermes-tracker-sync-cache-origin) "project" '(:state "all")))
        (hermes--promise-resolve
         new `((repo . ,(hermes-tracker-sync-tests--repo "project"))
               (todos . (,(hermes-tracker-sync-tests--todo 1 "New")))))
        (should (= calls 2))
        (should (equal (alist-get 'title
                                  (car (hermes-tracker-db-query-todos
                                        (hermes-tracker-sync-cache-origin)
                                        "project" '(:state "all"))))
                       "New"))))))

(ert-deftest hermes-tracker-sync-success-fences-read-started-during-mutation ()
  "Successful mutation settlement invalidates reads started during its HTTP."
  (hermes-tracker-sync-tests--with-state
    (let ((stale (hermes--promise-make)))
      (cl-letf (((symbol-function 'hermes-tracker--request-async)
                 (lambda (&rest _) stale)))
        (hermes-tracker-sync-note-mutation "project")
        (hermes-tracker-sync-todos "project")
        (hermes-tracker-sync-note-mutation "project")
        (hermes--promise-resolve
         stale `((repo . ,(hermes-tracker-sync-tests--repo "project"))
                 (todos . (,(hermes-tracker-sync-tests--todo 1 "Stale")))))
        (should-not (hermes-tracker-db-query-todos
                     (hermes-tracker-sync-cache-origin)
                     "project" '(:state "all")))))))

(ert-deftest hermes-tracker-sync-stale-rejection-is-ignored ()
  "A rejection from an invalidated request settles without reporting failure."
  (hermes-tracker-sync-tests--with-state
    (let ((pending (hermes--promise-make)) settled caught)
      (cl-letf (((symbol-function 'hermes-tracker--request-async)
                 (lambda (&rest _) pending)))
        (hermes--promise-then
         (hermes-tracker-sync-todos "project")
         (lambda (_) (setq settled t))
         (lambda (reason) (setq caught reason)))
        (hermes-tracker-sync-note-mutation "project")
        (hermes--promise-reject pending "obsolete offline")
        (should settled)
        (should-not caught)))))

(ert-deftest hermes-tracker-sync-invalid-page-rolls-back-repository-and-todos ()
  "One malformed TODO rolls back its repository page as one resource."
  (hermes-tracker-sync-tests--with-state
    (let ((origin (hermes-tracker-sync-cache-origin)))
      (hermes-tracker-db-upsert-repositories
       origin '(((slug . "project") (name . "Existing") (description . nil)))
       "before")
      (hermes-tracker-db-upsert-todos
       origin "project" (list (hermes-tracker-sync-tests--todo 1 "Existing")))
      (cl-letf (((symbol-function 'hermes-tracker--request-async)
                 (lambda (&rest _)
                   (hermes--promise-resolved
                    `((repo . ((slug . "project") (name . "Changed")
                               (description . nil)))
                      (todos . (,(hermes-tracker-sync-tests--todo 1 "Changed")
                                ((number . 2) (title . "Invalid")
                                 (status . "bogus") (priority . 0)
                                 (labels . nil)))))))))
        (hermes--promise-catch (hermes-tracker-sync-todos "project") #'ignore)
        (should (equal (alist-get 'name
                                  (car (hermes-tracker-db-list-repositories origin)))
                       "Existing"))
        (should (equal (alist-get 'title
                                  (car (hermes-tracker-db-query-todos
                                        origin "project" '(:state "all"))))
                       "Existing"))))))

(ert-deftest hermes-tracker-sync-error-metadata-failure-still-settles ()
  "A secondary DB error cannot strand the resource promise in flight."
  (hermes-tracker-sync-tests--with-state
    (let (caught)
      (cl-letf (((symbol-function 'hermes-tracker--request-async)
                 (lambda (&rest _) (hermes--promise-rejected "offline")))
                ((symbol-function 'hermes-tracker-db-record-sync-error)
                 (lambda (&rest _) (error "metadata unavailable"))))
        (hermes--promise-catch
         (hermes-tracker-sync-todos "project")
         (lambda (reason) (setq caught reason)))
        (should (equal caught "offline"))
        (should (= (hash-table-count hermes-tracker-sync--in-flight) 0))))))

(ert-deftest hermes-tracker-sync-failure-preserves-cache-and-records-error ()
  "A rejected request does not alter cached Tracker rows."
  (hermes-tracker-sync-tests--with-state
    (let ((origin (hermes-tracker-sync-cache-origin))
          events)
      (hermes-tracker-db-upsert-repositories
       origin (list (hermes-tracker-sync-tests--repo "project")) "before")
      (hermes-tracker-db-upsert-todos
       origin "project" (list (hermes-tracker-sync-tests--todo 1 "Existing")))
      (add-hook 'hermes-tracker-sync-changed-hook
                (lambda (&rest event) (push event events)))
      (cl-letf (((symbol-function 'hermes-tracker--request-async)
                 (lambda (&rest _) (hermes--promise-rejected "offline"))))
        (hermes--promise-catch (hermes-tracker-sync-todos "project") #'ignore)
        (should (equal (alist-get 'title
                                  (car (hermes-tracker-db-query-todos
                                        origin "project" '(:state "all"))))
                       "Existing"))
        (should (equal (plist-get
                        (hermes-tracker-db-sync-state origin "todos:project")
                        :last-error)
                       "offline"))
        (should-not events)))))

(ert-deftest hermes-tracker-sync-all-is-bounded-and-sequential ()
  "A full cycle fetches catalog then each repository without overlap."
  (hermes-tracker-sync-tests--with-state
    (let (paths)
      (cl-letf (((symbol-function 'hermes-tracker--request-async)
                 (lambda (_method path &rest _)
                   (setq paths (append paths (list path)))
                   (hermes--promise-resolved
                    (cond
                     ((equal path "/api/v1/repos")
                      (list (hermes-tracker-sync-tests--repo "alpha")
                            (hermes-tracker-sync-tests--repo "beta")))
                     ((equal path "/api/v1/labels")
                      '((labels . (((name . "bug") (color . "ff0000"))))
                        (fallback_color . "808080")))
                     ((string-match "/repos/\\([^/]+\\)/todos" path)
                      (let ((slug (match-string 1 path)))
                        `((repo . ,(hermes-tracker-sync-tests--repo slug))
                          (todos . (,(hermes-tracker-sync-tests--todo
                                     1 (capitalize slug))))))))))))
        (hermes-tracker-sync-all)
        (should (equal paths
                       '("/api/v1/repos" "/api/v1/labels"
                         "/api/v1/repos/alpha/todos?state=all"
                         "/api/v1/repos/beta/todos?state=all")))
        (should (= (length (hermes-tracker-db-list-labels
                            (hermes-tracker-sync-cache-origin)))
                   1))))))

(ert-deftest hermes-tracker-sync-disable-during-cycle-does-not-rearm-timer ()
  "Disabling background sync while active prevents timer resurrection."
  (hermes-tracker-sync-tests--with-state
    (let ((pending (hermes--promise-make))
          (scheduled 0))
      (cl-letf (((symbol-function 'hermes-tracker-sync-all)
                 (lambda () pending))
                ((symbol-function 'run-at-time)
                 (lambda (&rest _)
                   (cl-incf scheduled)
                   'fake-timer))
                ((symbol-function 'timerp) (lambda (_value) nil)))
        (hermes-tracker-sync-mode 1)
        (hermes-tracker-sync-mode -1)
        (hermes--promise-resolve pending nil)
        (should (= scheduled 0))))))

(provide 'hermes-tracker-sync-tests)
;;; hermes-tracker-sync-tests.el ends here
