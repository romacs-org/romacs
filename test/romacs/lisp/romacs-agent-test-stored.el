;;; romacs-agent-test-stored.el --- Sessions as a chat finds them, for tests -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A chat draws a session from SQLite: its records there and its
;; finished turns' rows written, as the move to SQLite and the workers
;; that write turn rows leave it.  Tests that write a session's journal
;; make it so before its chat is drawn.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-agent-events)
(require 'romacs-agent-turns)

(defmacro romacs-agent-test-with-store (&rest body)
  "BODY with a sessions database of its own, in a state root of its own:
no session's records or turns seen before it are taken for its own."
  (declare (indent 0))
  `(let* ((store-root (file-name-as-directory (make-temp-file "romacs-store-" t)))
          (process-environment (cons (concat "XDG_STATE_HOME=" store-root) process-environment))
          (romacs-agent-store--db nil) (romacs-agent-turns--made nil)
          (romacs-agent-events--made nil) (romacs-agent-events--imported (make-hash-table :test #'equal)))
     (unwind-protect (progn ,@body)
       (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
       (delete-directory store-root t))))

(defun romacs-agent-test-stored (session)
  "Make SESSION as a chat finds it: its journal's records in SQLite (the
journal kept) and its finished turns' rows written, as a worker writes
them.  Non-nil when its records are in SQLite."
  ;; Its id may have been another test's session's, imported elsewhere.
  (remhash (romacs-agent-id session) romacs-agent-events--imported)
  (when (romacs-agent-events-import session t)
    ;; Its row, which a chat restored shows it from.
    (romacs-agent-store-put session)
    (romacs-agent-turns-fill session)
    t))

(provide 'romacs-agent-test-stored)
;;; romacs-agent-test-stored.el ends here
