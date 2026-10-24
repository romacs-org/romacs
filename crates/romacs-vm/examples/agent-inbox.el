;;; agent-inbox.el --- Read-only inbox summary -*- lexical-binding: t; -*-
;;; Small real agent task, evaluated by VM2 without GNU reading the source.
;; No shell interpolation, no message bodies, no index mutations.
(defun agent-unread-inbox ()
  "Return a notmuch-backed inbox summary."
  (format "%s unread inbox messages"
          (car (process-lines "notmuch" "count" "--output=messages"
                              "tag:inbox and tag:unread"))))
(agent-unread-inbox)
