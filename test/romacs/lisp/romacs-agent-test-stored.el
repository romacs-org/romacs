;;; romacs-agent-test-stored.el --- Sessions as a chat finds them, for tests -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A chat draws a session from SQLite: its records there, its finished
;; turns' rows written and kept drawn, as the move to SQLite and the
;; workers that write turn rows leave it.  Tests that write a session's
;; journal make it so before its chat is drawn.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent)
(require 'romacs-agent-events)
(require 'romacs-agent-turns)
(require 'romacs-agent-ui)

(defmacro romacs-agent-test-with-store (&rest body)
  "BODY with a sessions database of its own, in a state root of its own:
no session's records or turns seen before it are taken for its own."
  (declare (indent 0))
  `(let* ((store-root (file-name-as-directory (make-temp-file "romacs-store-" t)))
          (process-environment (cons (concat "XDG_STATE_HOME=" store-root) process-environment))
          (romacs-agent-store--db nil) (romacs-agent-turns--made nil) (romacs-agent-drawn--made nil)
          (romacs-agent-events--made nil) (romacs-agent-events--imported (make-hash-table :test #'equal)))
     (unwind-protect (progn ,@body)
       (when (cdr romacs-agent-store--db) (ignore-errors (sqlite-close (cdr romacs-agent-store--db))))
       (delete-directory store-root t))))

(defun romacs-agent-test-stored (session)
  "Make SESSION as a chat finds it: its journal's records in SQLite (the
journal kept), its finished turns' rows written and each kept drawn, as
a worker keeps them.  Non-nil when its records are in SQLite."
  ;; Its id may have been another test's session's, imported elsewhere.
  (remhash (romacs-agent-id session) romacs-agent-events--imported)
  (when (romacs-agent-events-import session t)
    ;; Its row, which a chat restored shows it from.
    (romacs-agent-store-put session)
    (romacs-agent-turns-fill session)
    (dolist (row (plist-get (romacs-agent-turns-page session nil most-positive-fixnum) :rows))
      (romacs-agent-drawn-put (romacs-agent-turns--db) (romacs-agent-id session) (plist-get row :first-seq)
                              (romacs-agent-ui-draw-turn session row)))
    t))

(defun romacs-agent-test-drawn-live (session turns)
  "SESSION's chat as its records drew it live, every event in order, cut
to its last TURNS turns under the row that draws earlier ones when it had
more, as a chat opened later draws it: (TEXT . BUFFER)."
  (let ((buffer (romacs-agent-ui--make-chat session)))
    (with-current-buffer buffer
      (setq romacs-agent-ui--unfilled nil)
      (romacs-agent-ui--replay-from session nil)
      (let ((starts (romacs-agent-ui--input-starts)))
        (when (> (length starts) turns)
          (let ((inhibit-read-only t)) (delete-region (point-min) (nth (1- turns) starts)))
          (romacs-agent-ui--earlier-row)))
      (setf (romacs-agent-buffer session) nil)
      (cons (buffer-substring-no-properties (point-min) romacs-agent-ui--end) buffer))))

;; Every test that requires this: a chat's replay walking its records on
;; the editor fails it (`romacs-agent-ui--replay-over-function').
(setq romacs-agent-ui--replay-over-function
      (lambda (session drew turns)
        (error "Chat of %s replayed %d prompts from records, drawing %d" (romacs-agent-id session) drew turns)))

(provide 'romacs-agent-test-stored)
;;; romacs-agent-test-stored.el ends here
