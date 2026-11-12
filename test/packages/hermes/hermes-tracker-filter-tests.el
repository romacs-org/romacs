;;; hermes-tracker-filter-tests.el --- Tracker filter tests -*- lexical-binding: t; -*-

;;; Code:

(require 'ert)
(require 'hermes-tracker-db)
(require 'hermes-tracker-filter)

(ert-deftest hermes-tracker-filter-preserves-repeated-term-semantics ()
  "Labels remain AND terms and assignees remain OR terms."
  (should
   (equal (hermes-tracker-filter-parse
           "state:closed label:bug label:urgent assignee:alice assignee:bob q:\"token refresh\"")
          '(:state "closed"
            :labels ("bug" "urgent")
            :assignees ("alice" "bob")
            :query "token refresh"))))

(ert-deftest hermes-tracker-filter-defaults-to-all-and-bare-title ()
  "A bare query searches title across all states."
  (should (equal (hermes-tracker-filter-parse "offline cache")
                 '(:state "all" :labels nil :assignees nil
                   :query "offline cache"))))

(ert-deftest hermes-tracker-filter-title-query-reaches-sql-read-model ()
  "Parsed title search becomes a parameterized SQLite LIKE clause."
  (pcase-let ((`(,sql . ,parameters)
               (hermes-tracker-db--todo-query
                "origin" "repo" (hermes-tracker-filter-parse "q:cache"))))
    (should (string-match-p
             (regexp-quote "LOWER(title) LIKE LOWER(?)") sql))
    (should (member "%cache%" parameters))))

(ert-deftest hermes-tracker-filter-deduplicates-identical-terms ()
  "Duplicate terms do not change local query semantics."
  (should (equal (hermes-tracker-filter-parse
                  "label:bug label:bug assignee:dev assignee:dev")
                 '(:state "all" :labels ("bug") :assignees ("dev")
                   :query ""))))

(ert-deftest hermes-tracker-filter-rejects-invalid-state ()
  "Only the deployed lifecycle states and all are accepted."
  (should-error (hermes-tracker-filter-parse "state:active")
                :type 'user-error))

(ert-deftest hermes-tracker-filter-rejects-empty-and-unknown-fields ()
  "Malformed structured terms stop before a database query."
  (should-error (hermes-tracker-filter-parse "label:") :type 'user-error)
  (should-error (hermes-tracker-filter-parse "owner:dev") :type 'user-error))

(provide 'hermes-tracker-filter-tests)
;;; hermes-tracker-filter-tests.el ends here
