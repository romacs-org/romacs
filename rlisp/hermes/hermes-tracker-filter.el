;;; hermes-tracker-filter.el --- Parse local Tracker filters -*- lexical-binding: t; -*-

;; Copyright (C) 2026 Thanos Apollo

;; Author: Thanos Apollo <public@thanosapollo.org>
;; Keywords: tools
;; Package-Requires: ((emacs "30.1"))

;;; Commentary:

;; Parse the compact filter language used by cached Tracker TODO views.

;;; Code:

(require 'subr-x)

(defun hermes-tracker-filter--value (token prefix)
  "Return TOKEN value after PREFIX, rejecting an empty value."
  (let ((value (substring token (length prefix))))
    (when (string-empty-p value)
      (user-error "Filter %s requires a value" prefix))
    value))

(defun hermes-tracker-filter-parse (input)
  "Parse INPUT into a plist accepted by `hermes-tracker-db-query-todos'.

Supported terms are `state:open', `state:closed', `state:all', repeated
`label:' and `assignee:' terms, and `q:' or bare title words."
  (let ((state "all") labels assignees query)
    (dolist (token (split-string-shell-command input))
      (cond
       ((string-prefix-p "state:" token)
        (let ((value (hermes-tracker-filter--value token "state:")))
          (unless (member value '("open" "closed" "all"))
            (user-error "Unsupported Tracker state: %s" value))
          (setq state value)))
       ((string-prefix-p "label:" token)
        (push (hermes-tracker-filter--value token "label:") labels))
       ((string-prefix-p "assignee:" token)
        (push (hermes-tracker-filter--value token "assignee:") assignees))
       ((string-prefix-p "q:" token)
        (push (hermes-tracker-filter--value token "q:") query))
       ((string-match-p "\\`[[:alpha:]_-]+:" token)
        (user-error "Unsupported Tracker filter: %s" token))
       (t (push token query))))
    (list :state state
          :labels (delete-dups (nreverse labels))
          :assignees (delete-dups (nreverse assignees))
          :query (string-join (nreverse query) " "))))

(provide 'hermes-tracker-filter)
;;; hermes-tracker-filter.el ends here
