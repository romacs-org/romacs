;;; hermes-tracker-sync.el --- SQLite synchronization for Tracker -*- lexical-binding: t; -*-
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;; Copyright (C) 2026 Thanos Apollo

;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: tools, convenience
;; Package-Requires: ((emacs "29.1") (hermes "0.2.0"))

;;; Commentary:

;; Single-owner asynchronous synchronization from Tracker into SQLite.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'hermes-promise)
(require 'hermes-transport)
(require 'hermes-tracker-api)
(require 'hermes-tracker-db)

(defcustom hermes-tracker-sync-interval 300
  "Seconds between completed background Tracker synchronization cycles."
  :type '(integer 1)
  :group 'hermes-tracker)

(defvar hermes-tracker-sync-changed-hook nil
  "Hook run with cache origin and resource scope after a committed sync.")

(defvar hermes-tracker-sync--in-flight (make-hash-table :test #'equal)
  "Resource keys mapped to owned in-flight promise entries.")

(defvar hermes-tracker-sync--epochs (make-hash-table :test #'equal)
  "Resource epoch keys mapped to generation integers.")

(defvar hermes-tracker-sync--timer nil
  "Owned one-shot background synchronization timer.")

(defvar hermes-tracker-sync--mode-epoch 0
  "Generation preventing disabled background cycles from rearming.")

(defvar hermes-tracker-sync-mode nil
  "Non-nil while background Tracker synchronization is enabled.")

(defun hermes-tracker-sync--origin (base-url)
  "Return non-secret cache identity for BASE-URL, actor, and auth selectors."
  (prin1-to-string
   (list base-url
         (hermes-tracker--effective-actor)
         hermes-tracker-auth-source-user
         hermes-tracker-auth-source-port)))

(defun hermes-tracker-sync-cache-origin ()
  "Return non-secret identity of the configured Tracker cache namespace."
  (hermes-tracker-sync--origin (hermes-tracker--base-url)))

(defun hermes-tracker-sync--timestamp ()
  "Return current UTC timestamp in Tracker wire format."
  (format-time-string "%Y-%m-%dT%H:%M:%SZ" nil t))

(defun hermes-tracker-sync--scope-name (scope)
  "Return stable persisted name for resource SCOPE."
  (pcase scope
    ('(repos) "repos")
    ('(labels) "labels")
    (`(todos ,slug) (format "todos:%s" slug))
    (`(todo ,slug ,number) (format "todo:%s:%s" slug number))
    (_ (error "Unsupported Tracker sync scope: %S" scope))))

(defun hermes-tracker-sync--epoch-key (origin scope)
  "Return epoch key for ORIGIN resource SCOPE."
  (pcase scope
    (`(todos ,slug) (list origin 'repo slug))
    (`(todo ,slug ,_) (list origin 'repo slug))
    (`(,kind . ,_) (list origin kind))))

(defun hermes-tracker-sync--epoch (origin scope)
  "Return current epoch for ORIGIN SCOPE."
  (gethash (hermes-tracker-sync--epoch-key origin scope)
           hermes-tracker-sync--epochs 0))

(defun hermes-tracker-sync--owned-p (origin scope epoch)
  "Return non-nil when EPOCH still owns ORIGIN SCOPE."
  (= epoch (hermes-tracker-sync--epoch origin scope)))

(defun hermes-tracker-sync--bump (key)
  "Advance generation for epoch KEY."
  (puthash key (1+ (gethash key hermes-tracker-sync--epochs 0))
           hermes-tracker-sync--epochs))

(defun hermes-tracker-sync-note-mutation (slug)
  "Invalidate in-flight repository and TODO requests affected by mutating SLUG."
  (let ((origin
         (hermes-tracker-sync--origin
          (hermes-tracker--normalize-base-url hermes-tracker-url))))
    (hermes-tracker-sync--bump (list origin 'repos))
    (hermes-tracker-sync--bump (list origin 'repo slug))
    (maphash
     (lambda (key _entry)
       (pcase key
         (`(,entry-origin (repos))
          (when (equal entry-origin origin)
            (remhash key hermes-tracker-sync--in-flight)))
         (`(,entry-origin (,kind ,entry-slug . ,_))
          (when (and (equal entry-origin origin)
                     (memq kind '(todos todo))
                     (equal entry-slug slug))
            (remhash key hermes-tracker-sync--in-flight)))))
     (copy-hash-table hermes-tracker-sync--in-flight))))

(defun hermes-tracker-sync--error-string (reason)
  "Return stable display string for rejection REASON."
  (if (stringp reason) reason (format "%S" reason)))

(defun hermes-tracker-sync--field-present-p (object key)
  "Return non-nil when OBJECT has field KEY, including nil."
  (catch 'present
    (dolist (candidate (hermes-transport--key-candidates key))
      (unless (eq (hermes-transport--member-value object candidate)
                  hermes-transport--missing)
        (throw 'present t)))))

(defun hermes-tracker-sync--record-error (origin scope reason)
  "Best-effort record REASON for failed ORIGIN SCOPE."
  (condition-case nil
      (hermes-tracker-db-record-sync-error
       origin scope (hermes-tracker-sync--error-string reason))
    (error nil)))

(defun hermes-tracker-sync--run-changed-hook (origin scope)
  "Run change notification for ORIGIN SCOPE without breaking sync settlement."
  (condition-case error-data
      (run-hook-with-args 'hermes-tracker-sync-changed-hook origin scope)
    (error
     (message "Hermes Tracker sync hook failed: %s"
              (error-message-string error-data)))))

(defun hermes-tracker-sync--request (scope fetch apply)
  "Coalesce SCOPE request from FETCH and commit its value with APPLY."
  (let* ((origin (hermes-tracker-sync-cache-origin))
         (resource-key (list origin scope))
         (existing (gethash resource-key hermes-tracker-sync--in-flight)))
    (if existing
        (cdr existing)
      (let* ((epoch (hermes-tracker-sync--epoch origin scope))
             (result (hermes--promise-make))
             (entry (cons (make-symbol "sync-owner") result))
             (state-name (hermes-tracker-sync--scope-name scope))
             (source
              (condition-case error-data
                  (funcall fetch)
                ((error quit)
                 (hermes--promise-rejected
                  (error-message-string error-data))))))
        (puthash resource-key entry hermes-tracker-sync--in-flight)
        (hermes--promise-then
         source
         (lambda (value)
           (condition-case error-data
               (progn
                 (when (hermes-tracker-sync--owned-p origin scope epoch)
                   (funcall apply origin value)
                   (hermes-tracker-db-record-sync-success
                    origin state-name (hermes-tracker-sync--timestamp))
                   (hermes-tracker-sync--run-changed-hook origin scope))
                 (when (eq (gethash resource-key hermes-tracker-sync--in-flight)
                           entry)
                   (remhash resource-key hermes-tracker-sync--in-flight))
                 (hermes--promise-resolve result value))
             ((error quit)
              (let ((reason (error-message-string error-data)))
                (when (hermes-tracker-sync--owned-p origin scope epoch)
                  (hermes-tracker-sync--record-error origin state-name reason))
                (when (eq (gethash resource-key hermes-tracker-sync--in-flight)
                          entry)
                  (remhash resource-key hermes-tracker-sync--in-flight))
                (hermes--promise-reject result reason)))))
         (lambda (reason)
           (let ((owned (hermes-tracker-sync--owned-p origin scope epoch)))
             (when owned
               (hermes-tracker-sync--record-error origin state-name reason))
             (when (eq (gethash resource-key hermes-tracker-sync--in-flight)
                       entry)
               (remhash resource-key hermes-tracker-sync--in-flight))
             (if owned
                 (hermes--promise-reject result reason)
               (hermes--promise-resolve result nil)))))
        result))))

(defun hermes-tracker-sync-repositories ()
  "Return promise synchronizing repository summaries into SQLite."
  (let ((scope '(repos)))
    (hermes-tracker-sync--request
     scope
     (lambda () (hermes-tracker--request-async "GET" "/api/v1/repos"))
     (lambda (origin payload)
       (hermes-tracker-db-upsert-repositories
        origin (hermes-tracker--items payload)
        (hermes-tracker-sync--timestamp))))))

(defun hermes-tracker-sync-labels ()
  "Return promise synchronizing the label catalog into SQLite."
  (let ((scope '(labels)))
    (hermes-tracker-sync--request
     scope
     (lambda () (hermes-tracker--request-async "GET" "/api/v1/labels"))
     (lambda (origin payload)
       (hermes-tracker-db-replace-label-catalog
        origin payload (hermes-tracker-sync--timestamp))))))

(defun hermes-tracker-sync-todos (slug)
  "Return promise synchronizing all TODO summaries for repository SLUG."
  (let ((scope (list 'todos slug)))
    (hermes-tracker-sync--request
     scope
     (lambda ()
       (hermes-tracker--request-async
        "GET" (concat (hermes-tracker--api-path "repos" slug "todos")
                      "?state=all")))
     (lambda (origin payload)
       (let* ((repository (hermes-transport--get payload 'repo))
              (repository-slug (and repository
                                    (hermes-transport--get repository 'slug))))
         (unless (equal repository-slug slug)
           (error "Tracker TODO response repository mismatch for %s" slug))
         (unless (hermes-tracker-sync--field-present-p payload 'todos)
           (error "Tracker TODO response omitted todos"))
         (hermes-tracker-db-upsert-todo-page
          origin repository
          (hermes-tracker--items (hermes-transport--get payload 'todos))
          (hermes-tracker-sync--timestamp)))))))

(defun hermes-tracker-sync-todo (slug number)
  "Return promise synchronizing TODO NUMBER detail for repository SLUG."
  (let ((scope (list 'todo slug number)))
    (hermes-tracker-sync--request
     scope
     (lambda ()
       (hermes-tracker--request-async
        "GET" (hermes-tracker--api-path "repos" slug "todos" number)))
     (lambda (origin payload)
       (unless (and (equal (hermes-transport--get payload 'repo_slug) slug)
                    (equal (hermes-transport--get payload 'number) number))
         (error "Tracker TODO detail identity mismatch for %s#%s" slug number))
       (hermes-tracker-db-upsert-todo-detail
        origin slug payload (hermes-tracker-sync--timestamp))))))

(defun hermes-tracker-sync--ignore-error (promise)
  "Return PROMISE converted to nil when it rejects."
  (hermes--promise-catch promise (lambda (_reason) nil)))

(defun hermes-tracker-sync--repository-chain (repositories)
  "Return promise synchronizing REPOSITORIES sequentially."
  (let ((chain (hermes--promise-resolved nil)))
    (dolist (repository repositories)
      (let ((slug (hermes-transport--get repository 'slug)))
        (setq chain
              (hermes--promise-map
               chain
               (lambda (_)
                 (hermes-tracker-sync--ignore-error
                  (hermes-tracker-sync-todos slug)))))))
    chain))

(defun hermes-tracker-sync-all ()
  "Return promise for one bounded complete background synchronization cycle."
  (hermes--promise-map
   (hermes-tracker-sync-repositories)
   (lambda (repositories)
     (hermes--promise-map
      (hermes-tracker-sync--ignore-error (hermes-tracker-sync-labels))
      (lambda (_)
        (hermes--promise-map
         (hermes-tracker-sync--repository-chain
          (hermes-tracker--items repositories))
         (lambda (_) repositories)))))))

(defun hermes-tracker-sync--schedule (epoch)
  "Schedule next background cycle owned by mode EPOCH."
  (when (and hermes-tracker-sync-mode
             (= epoch hermes-tracker-sync--mode-epoch))
    (setq hermes-tracker-sync--timer
          (run-at-time
           (max 1 hermes-tracker-sync-interval) nil
           (lambda ()
             (when (and hermes-tracker-sync-mode
                        (= epoch hermes-tracker-sync--mode-epoch))
               (setq hermes-tracker-sync--timer nil)
               (hermes-tracker-sync--run-background epoch)))))))

(defun hermes-tracker-sync--run-background (epoch)
  "Run one background cycle owned by mode EPOCH."
  (let ((promise
         (condition-case error-data
             (hermes-tracker-sync-all)
           ((error quit)
            (hermes--promise-rejected (error-message-string error-data))))))
    (hermes--promise-then
     promise
     (lambda (_value) (hermes-tracker-sync--schedule epoch))
     (lambda (reason)
       (when (and hermes-tracker-sync-mode
                  (= epoch hermes-tracker-sync--mode-epoch))
         (message "Hermes Tracker sync failed: %s"
                  (hermes-tracker-sync--error-string reason)))
       (hermes-tracker-sync--schedule epoch)))))

;;;###autoload
(define-minor-mode hermes-tracker-sync-mode
  "Synchronize Tracker into SQLite after each settled one-shot cycle."
  :global t
  :group 'hermes-tracker
  (cl-incf hermes-tracker-sync--mode-epoch)
  (when (timerp hermes-tracker-sync--timer)
    (cancel-timer hermes-tracker-sync--timer))
  (setq hermes-tracker-sync--timer nil)
  (when hermes-tracker-sync-mode
    (hermes-tracker-sync--run-background hermes-tracker-sync--mode-epoch)))

(provide 'hermes-tracker-sync)
;;; hermes-tracker-sync.el ends here
