;;; hermes-tracker-db.el --- SQLite read model for Tracker -*- lexical-binding: t; -*-
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Copyright (C) 2026 Thanos Apollo

;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: tools, convenience
;; Package-Requires: ((emacs "29.1") (hermes "0.2.0"))

;;; Commentary:

;; Local SQLite read model for Hermes Tracker.  Tracker remains mutation
;; authority; this library stores API results and serves local client reads.

;;; Code:

(require 'cl-lib)
(require 'json)
(require 'sqlite)
(require 'subr-x)
(require 'hermes-transport)

(define-error 'hermes-tracker-db-error "Hermes Tracker database error")
(define-error 'hermes-tracker-db-schema-error
  "Unsupported Hermes Tracker database schema" 'hermes-tracker-db-error)

(defconst hermes-tracker-db--schema-version 1
  "Current Tracker cache schema version.")

(defconst hermes-tracker-db--missing-field (make-symbol "missing-field")
  "Marker for an optional API field omitted from a partial response.")

(defconst hermes-tracker-db--closure-provenance-keys
  '(closing_commit closing_source_repo closing_source_ref closed_by closed_at)
  "Detail fields invalidated when a TODO reopens.")

(defcustom hermes-tracker-db-file
  (locate-user-emacs-file "hermes-tracker/tracker.db")
  "File storing the local Hermes Tracker read model."
  :type 'file
  :group 'hermes-tracker)

(defvar hermes-tracker-db--connection nil
  "Open SQLite connection for the Tracker read model.")

(defconst hermes-tracker-db--schema
  '("CREATE TABLE IF NOT EXISTS repositories (
       cache_origin TEXT NOT NULL,
       slug TEXT NOT NULL,
       name TEXT NOT NULL,
       description TEXT,
       synced_at TEXT NOT NULL,
       PRIMARY KEY (cache_origin, slug))"
    "CREATE TABLE IF NOT EXISTS todos (
       cache_origin TEXT NOT NULL,
       repo_slug TEXT NOT NULL,
       number INTEGER NOT NULL,
       title TEXT NOT NULL,
       status TEXT NOT NULL,
       priority INTEGER NOT NULL DEFAULT 0,
       assignee TEXT,
       author TEXT,
       updated_at TEXT,
       closed_at TEXT,
       detail_json TEXT,
       detail_synced_at TEXT,
       PRIMARY KEY (cache_origin, repo_slug, number),
       FOREIGN KEY (cache_origin, repo_slug)
         REFERENCES repositories (cache_origin, slug) ON DELETE CASCADE)"
    "CREATE TABLE IF NOT EXISTS todo_labels (
       cache_origin TEXT NOT NULL,
       repo_slug TEXT NOT NULL,
       todo_number INTEGER NOT NULL,
       label TEXT NOT NULL,
       PRIMARY KEY (cache_origin, repo_slug, todo_number, label),
       FOREIGN KEY (cache_origin, repo_slug, todo_number)
         REFERENCES todos (cache_origin, repo_slug, number) ON DELETE CASCADE)"
    "CREATE INDEX IF NOT EXISTS todos_lookup
       ON todos (cache_origin, repo_slug, status, number)"
    "CREATE INDEX IF NOT EXISTS todo_labels_lookup
       ON todo_labels (cache_origin, repo_slug, label, todo_number)"
    "CREATE TABLE IF NOT EXISTS label_catalog (
       cache_origin TEXT NOT NULL,
       name TEXT NOT NULL,
       color TEXT,
       synced_at TEXT NOT NULL,
       PRIMARY KEY (cache_origin, name))"
    "CREATE TABLE IF NOT EXISTS label_catalog_meta (
       cache_origin TEXT PRIMARY KEY,
       fallback_color TEXT NOT NULL,
       synced_at TEXT NOT NULL)"
    "CREATE TABLE IF NOT EXISTS sync_state (
       cache_origin TEXT NOT NULL,
       scope TEXT NOT NULL,
       last_success_at TEXT,
       last_error TEXT,
       PRIMARY KEY (cache_origin, scope))")
  "SQL creating the Tracker read model.")

(defun hermes-tracker-db--initialize (database)
  "Initialize and validate DATABASE schema."
  (with-sqlite-transaction database
    (sqlite-execute database
                    "CREATE TABLE IF NOT EXISTS schema_meta (
                       singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                       version INTEGER NOT NULL)")
    (let ((row (car (sqlite-select
                     database
                     "SELECT version FROM schema_meta WHERE singleton = 1"))))
      (cond
       ((null row)
        (sqlite-execute database
                        "INSERT INTO schema_meta (singleton, version) VALUES (1, ?)"
                        (list hermes-tracker-db--schema-version)))
       ((/= (car row) hermes-tracker-db--schema-version)
        (signal 'hermes-tracker-db-schema-error (list (car row))))))
    (dolist (statement hermes-tracker-db--schema)
      (sqlite-execute database statement))))

(defun hermes-tracker-db-open ()
  "Open and return the local Tracker database."
  (unless (sqlite-available-p)
    (signal 'hermes-tracker-db-error '("This Emacs lacks SQLite support")))
  (unless (and hermes-tracker-db--connection
               (sqlitep hermes-tracker-db--connection))
    (let* ((path (expand-file-name hermes-tracker-db-file))
           (directory (file-name-directory path)))
      (unless (file-directory-p directory)
        (make-directory directory t)
        (set-file-modes directory #o700))
      (unless (zerop (logand (file-modes directory) #o077))
        (user-error "Tracker cache directory must not be group/world accessible: %s"
                    directory))
      (when (file-symlink-p path)
        (user-error "Tracker cache must not be a symbolic link: %s" path))
      (when (and (file-exists-p path) (not (file-regular-p path)))
        (user-error "Tracker cache must be a regular file: %s" path))
      (let ((database (sqlite-open path)))
        (set-file-modes path #o600)
        (condition-case error-data
            (progn
              (sqlite-execute database "PRAGMA foreign_keys = ON")
              (hermes-tracker-db--initialize database)
              (setq hermes-tracker-db--connection database))
          (error
           (sqlite-close database)
           (signal (car error-data) (cdr error-data)))))))
  hermes-tracker-db--connection)

(defun hermes-tracker-db-close ()
  "Close the local Tracker database."
  (when (and hermes-tracker-db--connection
             (sqlitep hermes-tracker-db--connection))
    (sqlite-close hermes-tracker-db--connection))
  (setq hermes-tracker-db--connection nil))

(add-hook 'kill-emacs-hook #'hermes-tracker-db-close)

(defun hermes-tracker-db--get (object key)
  "Return OBJECT value for KEY."
  (hermes-transport--get object key))

(defun hermes-tracker-db--has-key-p (object key)
  "Return non-nil when KEY is present in OBJECT, including a nil value."
  (let* ((name (symbol-name key))
         (candidates (list key name (intern (concat ":" name)))))
    (cond
     ((hash-table-p object)
      (let ((missing (make-symbol "missing")))
        (seq-some (lambda (candidate)
                    (not (eq (gethash candidate object missing) missing)))
                  candidates)))
     ((and (listp object) (seq-every-p #'consp object))
      (seq-some (lambda (candidate) (assoc candidate object)) candidates))
     ((listp object)
      (seq-some (lambda (candidate) (plist-member object candidate))
                candidates)))))

(defun hermes-tracker-db--required-string (object key)
  "Return OBJECT required string KEY or signal an error."
  (let ((value (hermes-tracker-db--get object key)))
    (unless (and (stringp value) (not (string-empty-p value)))
      (signal 'hermes-tracker-db-error
              (list (format "Tracker record requires %s" key))))
    value))

(defun hermes-tracker-db--optional-string (object key)
  "Return OBJECT optional string KEY or nil."
  (let ((value (hermes-tracker-db--get object key)))
    (and (stringp value) (not (string-empty-p value)) value)))

(defun hermes-tracker-db--partial-string (object key)
  "Return OBJECT string KEY, nil, or the missing-field marker."
  (if (hermes-tracker-db--has-key-p object key)
      (hermes-tracker-db--optional-string object key)
    hermes-tracker-db--missing-field))

(defun hermes-tracker-db--repository-record (repository)
  "Return normalized plain record for REPOSITORY."
  (let ((slug (hermes-tracker-db--required-string repository 'slug)))
    (list slug
          (or (hermes-tracker-db--optional-string repository 'name) slug)
          (hermes-tracker-db--optional-string repository 'description))))

(defun hermes-tracker-db--labels (todo)
  "Return normalized label strings from TODO."
  (unless (hermes-tracker-db--has-key-p todo 'labels)
    (signal 'hermes-tracker-db-error '("Tracker TODO requires labels")))
  (let ((labels (hermes-tracker-db--get todo 'labels)))
    (unless (or (null labels) (listp labels) (vectorp labels))
      (signal 'hermes-tracker-db-error '("Tracker TODO labels must be a list")))
    (let ((values (if (vectorp labels) (append labels nil) labels)))
      (unless (seq-every-p (lambda (label)
                             (and (stringp label) (not (string-empty-p label))))
                           values)
        (signal 'hermes-tracker-db-error '("Tracker TODO labels must be strings")))
      (sort (delete-dups (copy-sequence values)) #'string<))))

(defun hermes-tracker-db--todo-record (todo)
  "Return normalized plain record for TODO."
  (let ((number (hermes-tracker-db--get todo 'number))
        (priority (hermes-tracker-db--get todo 'priority))
        (status (hermes-tracker-db--required-string todo 'status)))
    (unless (and (integerp number) (> number 0))
      (signal 'hermes-tracker-db-error '("Tracker TODO requires a positive number")))
    (unless (integerp priority)
      (signal 'hermes-tracker-db-error '("Tracker TODO priority must be an integer")))
    (unless (member status '("open" "closed"))
      (signal 'hermes-tracker-db-error
              (list (format "Unsupported Tracker TODO status: %s" status))))
    (list number
          (hermes-tracker-db--required-string todo 'title)
          status
          priority
          (hermes-tracker-db--partial-string todo 'assignee)
          (hermes-tracker-db--partial-string todo 'author)
          (hermes-tracker-db--partial-string todo 'updated_at)
          (hermes-tracker-db--partial-string todo 'closed_at)
          (hermes-tracker-db--labels todo))))

(defun hermes-tracker-db--upsert-repository-row
    (database origin record synced-at)
  "Upsert normalized repository RECORD for ORIGIN into DATABASE."
  (sqlite-execute
   database
   "INSERT INTO repositories
      (cache_origin, slug, name, description, synced_at)
    VALUES (?, ?, ?, ?, ?)
    ON CONFLICT (cache_origin, slug) DO UPDATE SET
      name = excluded.name,
      description = excluded.description,
      synced_at = excluded.synced_at"
   (append (list origin) record (list synced-at))))

(defun hermes-tracker-db--prune-repositories (database origin records)
  "Delete DATABASE repositories absent from complete ORIGIN RECORDS."
  (if records
      (sqlite-execute
       database
       (format "DELETE FROM repositories
                 WHERE cache_origin = ? AND slug NOT IN (%s)"
               (hermes-tracker-db--placeholders (length records)))
       (cons origin (mapcar #'car records)))
    (sqlite-execute database
                    "DELETE FROM repositories WHERE cache_origin = ?"
                    (list origin))))

(defun hermes-tracker-db-upsert-repositories (origin repositories synced-at)
  "Replace complete ORIGIN REPOSITORIES snapshot observed at SYNCED-AT."
  (let ((records (mapcar #'hermes-tracker-db--repository-record repositories))
        (database (hermes-tracker-db-open)))
    (with-sqlite-transaction database
      (dolist (record records)
        (hermes-tracker-db--upsert-repository-row
         database origin record synced-at))
      (hermes-tracker-db--prune-repositories database origin records))))

(defun hermes-tracker-db-list-repositories (origin)
  "Return cached repositories for ORIGIN."
  (mapcar (lambda (row)
            `((slug . ,(nth 0 row))
              (name . ,(nth 1 row))
              (description . ,(nth 2 row))))
          (sqlite-select
           (hermes-tracker-db-open)
           "SELECT slug, name, description FROM repositories
             WHERE cache_origin = ? ORDER BY slug"
           (list origin))))

(defun hermes-tracker-db-list-repository-summaries (origin)
  "Return ORIGIN repositories with TODO counts derived from SQLite."
  (mapcar
   (lambda (row)
     `((slug . ,(nth 0 row))
       (name . ,(nth 1 row))
       (description . ,(nth 2 row))
       (summary . ((status_counts . ((open . ,(nth 3 row))
                                     (closed . ,(nth 4 row))))))))
   (sqlite-select
    (hermes-tracker-db-open)
    "SELECT r.slug, r.name, r.description,
            COALESCE(SUM(CASE WHEN t.status = 'open' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN t.status = 'closed' THEN 1 ELSE 0 END), 0)
       FROM repositories AS r
       LEFT JOIN todos AS t
         ON t.cache_origin = r.cache_origin AND t.repo_slug = r.slug
      WHERE r.cache_origin = ?
      GROUP BY r.slug, r.name, r.description
      ORDER BY r.slug"
    (list origin))))

(defun hermes-tracker-db--label-record (label)
  "Return normalized plain record for LABEL."
  (list (hermes-tracker-db--required-string label 'name)
        (hermes-tracker-db--required-string label 'color)))

(defun hermes-tracker-db-replace-label-catalog (origin catalog synced-at)
  "Replace ORIGIN label CATALOG observed at SYNCED-AT."
  (unless (hermes-tracker-db--has-key-p catalog 'labels)
    (signal 'hermes-tracker-db-error '("Tracker label catalog requires labels")))
  (let ((records (mapcar #'hermes-tracker-db--label-record
                         (hermes-tracker-db--get catalog 'labels)))
        (fallback (hermes-tracker-db--required-string
                   catalog 'fallback_color))
        (database (hermes-tracker-db-open)))
    (with-sqlite-transaction database
      (sqlite-execute database
                      "DELETE FROM label_catalog WHERE cache_origin = ?"
                      (list origin))
      (dolist (record records)
        (sqlite-execute database
                        "INSERT INTO label_catalog
                          (cache_origin, name, color, synced_at)
                         VALUES (?, ?, ?, ?)"
                        (append (list origin) record (list synced-at))))
      (sqlite-execute database
                      "INSERT INTO label_catalog_meta
                        (cache_origin, fallback_color, synced_at)
                       VALUES (?, ?, ?)
                       ON CONFLICT (cache_origin) DO UPDATE SET
                         fallback_color = excluded.fallback_color,
                         synced_at = excluded.synced_at"
                      (list origin fallback synced-at)))))

(defun hermes-tracker-db-list-labels (origin)
  "Return cached label catalog for ORIGIN."
  (mapcar (lambda (row)
            `((name . ,(nth 0 row))
              (color . ,(nth 1 row))))
          (sqlite-select (hermes-tracker-db-open)
                         "SELECT name, color FROM label_catalog
                           WHERE cache_origin = ? ORDER BY name COLLATE NOCASE, name"
                         (list origin))))

(defun hermes-tracker-db-label-fallback-color (origin)
  "Return cached fallback label color for ORIGIN, or nil."
  (caar (sqlite-select (hermes-tracker-db-open)
                       "SELECT fallback_color FROM label_catalog_meta
                         WHERE cache_origin = ?"
                       (list origin))))

(defun hermes-tracker-db-record-sync-success (origin scope synced-at)
  "Record successful ORIGIN SCOPE synchronization at SYNCED-AT."
  (sqlite-execute
   (hermes-tracker-db-open)
   "INSERT INTO sync_state (cache_origin, scope, last_success_at, last_error)
      VALUES (?, ?, ?, NULL)
    ON CONFLICT (cache_origin, scope) DO UPDATE SET
      last_success_at = excluded.last_success_at,
      last_error = NULL"
   (list origin scope synced-at)))

(defun hermes-tracker-db-record-sync-error (origin scope error)
  "Record ERROR for failed ORIGIN SCOPE synchronization."
  (sqlite-execute
   (hermes-tracker-db-open)
   "INSERT INTO sync_state (cache_origin, scope, last_success_at, last_error)
      VALUES (?, ?, NULL, ?)
    ON CONFLICT (cache_origin, scope) DO UPDATE SET
      last_error = excluded.last_error"
   (list origin scope error)))

(defun hermes-tracker-db-sync-state (origin scope)
  "Return synchronization state plist for ORIGIN SCOPE, or nil."
  (when-let* ((row (car (sqlite-select
                         (hermes-tracker-db-open)
                         "SELECT last_success_at, last_error FROM sync_state
                           WHERE cache_origin = ? AND scope = ?"
                         (list origin scope)))))
    (list :last-success-at (nth 0 row) :last-error (nth 1 row))))

(defun hermes-tracker-db--resolve-todo-record
    (database origin slug record)
  "Resolve partial RECORD from existing DATABASE ORIGIN SLUG state."
  (let ((existing
         (car (sqlite-select
               database
               "SELECT assignee, author, updated_at, closed_at FROM todos
                 WHERE cache_origin = ? AND repo_slug = ? AND number = ?"
               (list origin slug (car record))))))
    (cl-loop for value in record
             for index from 0
             collect
             (cond
              ((and (= index 7) (equal (nth 2 record) "open")) nil)
              ((eq value hermes-tracker-db--missing-field)
               (nth (- index 4) existing))
              (t value)))))

(defun hermes-tracker-db--upsert-todo-row (database origin slug record)
  "Upsert one normalized TODO RECORD into DATABASE for ORIGIN SLUG."
  (setq record (hermes-tracker-db--resolve-todo-record
                database origin slug record))
  (sqlite-execute
   database
   "INSERT INTO todos
      (cache_origin, repo_slug, number, title, status, priority,
       assignee, author, updated_at, closed_at)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT (cache_origin, repo_slug, number) DO UPDATE SET
      title = excluded.title,
      status = excluded.status,
      priority = excluded.priority,
      assignee = excluded.assignee,
      author = excluded.author,
      updated_at = COALESCE(excluded.updated_at, todos.updated_at),
      closed_at = excluded.closed_at"
   (append (list origin slug) (seq-take record 8)))
  (when (equal (nth 2 record) "open")
    (let ((detail-row
           (car (sqlite-select
                 database
                 "SELECT detail_json FROM todos
                   WHERE cache_origin = ? AND repo_slug = ? AND number = ?"
                 (list origin slug (car record))))))
      (when (car detail-row)
        (let* ((detail (json-parse-string
                        (car detail-row) :object-type 'alist :array-type 'list
                        :null-object nil :false-object nil))
               (sanitized (hermes-tracker-db--without-closure-provenance
                           detail)))
          (sqlite-execute
           database
           "UPDATE todos SET detail_json = ?
             WHERE cache_origin = ? AND repo_slug = ? AND number = ?"
           (list (json-encode sanitized) origin slug (car record)))))))
  (let ((number (car record))
        (labels (nth 8 record)))
    (sqlite-execute database
                    "DELETE FROM todo_labels
                      WHERE cache_origin = ? AND repo_slug = ? AND todo_number = ?"
                    (list origin slug number))
    (dolist (label labels)
      (sqlite-execute database
                      "INSERT INTO todo_labels
                        (cache_origin, repo_slug, todo_number, label)
                       VALUES (?, ?, ?, ?)"
                      (list origin slug number label)))))

(defun hermes-tracker-db-upsert-todos (origin slug todos)
  "Upsert summary TODOS for ORIGIN repository SLUG."
  (let ((records (mapcar #'hermes-tracker-db--todo-record todos))
        (database (hermes-tracker-db-open)))
    (with-sqlite-transaction database
      (dolist (record records)
        (hermes-tracker-db--upsert-todo-row database origin slug record)))))

(defun hermes-tracker-db-upsert-todo-page
    (origin repository todos synced-at)
  "Atomically replace ORIGIN REPOSITORY summary TODOS at SYNCED-AT."
  (let ((repository-record (hermes-tracker-db--repository-record repository))
        (todo-records (mapcar #'hermes-tracker-db--todo-record todos))
        (database (hermes-tracker-db-open)))
    (with-sqlite-transaction database
      (hermes-tracker-db--upsert-repository-row
       database origin repository-record synced-at)
      (dolist (record todo-records)
        (hermes-tracker-db--upsert-todo-row
         database origin (car repository-record) record))
      (let ((slug (car repository-record)))
        (if todo-records
            (sqlite-execute
             database
             (format "DELETE FROM todos
                       WHERE cache_origin = ? AND repo_slug = ?
                         AND number NOT IN (%s)"
                     (hermes-tracker-db--placeholders
                      (length todo-records)))
             (append (list origin slug) (mapcar #'car todo-records)))
          (sqlite-execute
           database
           "DELETE FROM todos WHERE cache_origin = ? AND repo_slug = ?"
           (list origin slug)))))))

(defun hermes-tracker-db-upsert-todo-detail (origin slug todo synced-at)
  "Upsert detailed TODO for ORIGIN repository SLUG at SYNCED-AT."
  (let* ((record (hermes-tracker-db--todo-record todo))
         (normalized (json-parse-string
                      (json-encode todo) :object-type 'alist :array-type 'list
                      :null-object nil :false-object nil))
         (detail (if (equal (nth 2 record) "open")
                     (hermes-tracker-db--without-closure-provenance normalized)
                   normalized))
         (json (json-encode detail))
         (database (hermes-tracker-db-open)))
    (with-sqlite-transaction database
      (hermes-tracker-db--upsert-todo-row database origin slug record)
      (sqlite-execute database
                      "UPDATE todos SET detail_json = ?, detail_synced_at = ?
                        WHERE cache_origin = ? AND repo_slug = ? AND number = ?"
                      (list json synced-at origin slug (car record))))))

(defun hermes-tracker-db--todo-labels (database origin slug number)
  "Return labels from DATABASE for ORIGIN SLUG TODO NUMBER."
  (mapcar #'car
          (sqlite-select database
                         "SELECT label FROM todo_labels
                           WHERE cache_origin = ? AND repo_slug = ? AND todo_number = ?
                           ORDER BY label"
                         (list origin slug number))))

(defun hermes-tracker-db--row-to-todo (database origin slug row)
  "Return API-shaped TODO from DATABASE ROW for ORIGIN SLUG."
  (let ((number (nth 0 row)))
    `((number . ,number)
      (title . ,(nth 1 row))
      (status . ,(nth 2 row))
      (priority . ,(nth 3 row))
      (assignee . ,(nth 4 row))
      (author . ,(nth 5 row))
      (updated_at . ,(nth 6 row))
      (closed_at . ,(nth 7 row))
      (labels . ,(hermes-tracker-db--todo-labels database origin slug number)))))

(defun hermes-tracker-db--placeholders (count)
  "Return COUNT comma-separated SQL placeholders."
  (string-join (make-list count "?") ", "))

(defun hermes-tracker-db--escape-like (value)
  "Escape VALUE for a SQLite LIKE pattern."
  (let ((escaped (replace-regexp-in-string "\\\\" "\\\\" value nil t)))
    (setq escaped (replace-regexp-in-string "%" "\\%" escaped nil t)
          escaped (replace-regexp-in-string "_" "\\_" escaped nil t))
    (concat "%" escaped "%")))

(defun hermes-tracker-db--filter-values (values name validator)
  "Return unique VALUES accepted by VALIDATOR, or signal for filter NAME."
  (unless (or (null values) (listp values))
    (signal 'hermes-tracker-db-error
            (list (format "Tracker %s filter must be a list" name))))
  (unless (seq-every-p validator values)
    (signal 'hermes-tracker-db-error
            (list (format "Invalid Tracker %s filter" name))))
  (delete-dups (copy-sequence values)))

(defun hermes-tracker-db--todo-query (origin slug filters)
  "Return SQL and parameters for ORIGIN SLUG FILTERS."
  (let* ((state (or (plist-get filters :state) "open"))
         (raw-query (plist-get filters :query))
         (query (and (stringp raw-query) (string-trim raw-query)))
         (labels
          (hermes-tracker-db--filter-values
           (plist-get filters :labels) "label"
           (lambda (label)
             (and (stringp label)
                  (string-match-p "\\`[a-z0-9][a-z0-9._-]\\{0,49\\}\\'"
                                  label)))))
         (assignees
          (hermes-tracker-db--filter-values
           (plist-get filters :assignees) "assignee"
           (lambda (assignee)
             (and (stringp assignee) (not (string-empty-p assignee))
                  (<= (length assignee) 200)))))
         (clauses '("cache_origin = ?" "repo_slug = ?"))
         (params (list origin slug)))
    (unless (member state '("open" "closed" "all"))
      (signal 'hermes-tracker-db-error
              (list (format "Unsupported Tracker state filter: %s" state))))
    (when (and raw-query (not (stringp raw-query)))
      (signal 'hermes-tracker-db-error '("Tracker title query must be a string")))
    (when (and state (not (equal state "all")))
      (setq clauses (append clauses '("status = ?"))
            params (append params (list state))))
    (when (and (stringp query) (not (string-empty-p query)))
      (setq clauses (append clauses '("LOWER(title) LIKE LOWER(?) ESCAPE '\\'"))
            params (append params (list (hermes-tracker-db--escape-like query)))))
    (when labels
      (setq clauses
            (append clauses
                    (list
                     (format
                      "number IN (
                         SELECT todo_number FROM todo_labels
                          WHERE cache_origin = ? AND repo_slug = ?
                            AND label IN (%s)
                          GROUP BY todo_number
                         HAVING COUNT(DISTINCT label) = ?)"
                      (hermes-tracker-db--placeholders (length labels)))))
            params (append params (list origin slug) labels (list (length labels)))))
    (when assignees
      (setq clauses
            (append clauses
                    (list (format "assignee IN (%s)"
                                  (hermes-tracker-db--placeholders
                                   (length assignees)))))
            params (append params assignees)))
    (cons (format
           "SELECT number, title, status, priority, assignee, author, updated_at,
                   closed_at
              FROM todos WHERE %s ORDER BY number"
           (string-join clauses " AND "))
          params)))

(defun hermes-tracker-db-query-todos (origin slug filters)
  "Return ORIGIN SLUG TODO summaries matching FILTERS."
  (let* ((database (hermes-tracker-db-open))
         (query (hermes-tracker-db--todo-query origin slug filters)))
    (mapcar (lambda (row)
              (hermes-tracker-db--row-to-todo database origin slug row))
            (sqlite-select database (car query) (cdr query)))))

(defun hermes-tracker-db--merge-summary (detail summary)
  "Return DETAIL with current SUMMARY fields replacing old values."
  (let ((keys (mapcar #'car summary)))
    (append summary
            (cl-remove-if (lambda (pair) (memq (car pair) keys)) detail))))

(defun hermes-tracker-db--without-closure-provenance (todo)
  "Return TODO without closure provenance invalidated by reopen."
  (cl-remove-if
   (lambda (pair)
     (memq (car pair) hermes-tracker-db--closure-provenance-keys))
   todo))

(defun hermes-tracker-db-get-todo (origin slug number)
  "Return cached ORIGIN SLUG TODO NUMBER, or nil."
  (let* ((database (hermes-tracker-db-open))
         (row (car (sqlite-select
                    database
                    "SELECT number, title, status, priority, assignee, author,
                            updated_at, closed_at, detail_json
                       FROM todos
                      WHERE cache_origin = ? AND repo_slug = ? AND number = ?"
                    (list origin slug number)))))
    (when row
      (let* ((summary (hermes-tracker-db--row-to-todo
                       database origin slug (seq-take row 8)))
             (detail-json (nth 8 row))
             (detail (and detail-json
                          (json-parse-string detail-json :object-type 'alist
                                             :array-type 'list
                                             :null-object nil :false-object nil))))
        (let ((todo (hermes-tracker-db--merge-summary detail summary)))
          (if (equal (alist-get 'status todo) "open")
              (hermes-tracker-db--without-closure-provenance todo)
            todo))))))

(provide 'hermes-tracker-db)
;;; hermes-tracker-db.el ends here
