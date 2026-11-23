;;; romacs-agent-idle.el --- Compact an idle thread while its cache is warm  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A large idle thread whose harness supports it is compacted before its prompt
;; cache expires, as T3 Code does (IdleCompaction.ts): the compaction reads
;; the context at the cached price, and the next prompt starts from the
;; summary instead of paying a cold read of the whole context.  Only a
;; thread worth it (at least 200k tokens of context, Claude Code's own
;; idle-compaction floor), whose turn ended, with nothing waiting on it or
;; running under it, once per idle spell, never a delegated task's.

;;; Code:

(require 'romacs-agent)

(defcustom romacs-agent-idle-compaction t
  "Whether a large idle thread is compacted just before its
prompt cache expires: its last request's time and the cache's life that
request reported (an hour, or five minutes)."
  :type 'boolean :group 'romacs-agent)

(defconst romacs-agent-idle--min-tokens 200000
  "The least context worth compacting: below it a cold read is cheap.")

(defconst romacs-agent-idle--lead 120
  "Seconds before the cache expires to compact: time for the compaction
to start while it is still warm.")

(defun romacs-agent-idle--busy-p (session)
  "Whether SESSION has an approval, queued prompt, or active worker."
  (or (> (hash-table-count (romacs-agent-approvals session)) 0)
      (romacs-agent-queue session)
      (and (romacs-agent-subagents session)
           (cl-some (lambda (row) (equal (plist-get row :state) "running"))
                    (hash-table-values (romacs-agent-subagents session))))
      (cl-some (lambda (child) (memq (romacs-agent-state child) '(starting running waiting-approval)))
               (romacs-agent-children session))))

(defun romacs-agent-idle--warm-p (state context now)
  "Whether STATE's cache for CONTEXT is worth compacting at NOW."
  (let ((expires (plist-get state :cache-expires)))
    (and (>= (or context 0) romacs-agent-idle--min-tokens)
         (not (plist-get state :compact-turn))
         (numberp expires)
         (<= (- expires romacs-agent-idle--lead) now)
         (< now expires))))

(defun romacs-agent-idle-due-p (session now)
  "Whether SESSION's thread is to be compacted at NOW (`float-time')."
  (and romacs-agent-idle-compaction
       (romacs-agent--idle-compact-prompt session)
       (not (plist-get (romacs-agent-options session) :parent))
       (eq (romacs-agent-loaded (romacs-agent-id session)) session)
       (eq (romacs-agent-state session) 'idle)
       (not (or (romacs-agent-failed session) (romacs-agent-settled-p session)
                (romacs-agent--per-turn-p session)))
       (romacs-agent-idle--warm-p (romacs-agent-driver-state session) (romacs-agent-context-used session) now)
       (romacs-agent--open-p session nil)
       (not (romacs-agent-idle--busy-p session))))

(defun romacs-agent-idle--compact (session)
  "Send SESSION's harness the prompt that compacts its thread."
  (romacs-agent-send session (romacs-agent--idle-compact-prompt session)))

(defun romacs-agent-idle--tick ()
  "Compact every thread due now."
  (let ((now (float-time)))
    (dolist (session (romacs-agent-sessions))
      (when (romacs-agent-idle-due-p session now)
        (condition-case err (romacs-agent-idle--compact session)
          (error (message "Romacs could not compact %s: %s" (romacs-agent-title session) (error-message-string err))))))))

(defvar romacs-agent-idle--timer nil)

(unless (timerp romacs-agent-idle--timer)
  (setq romacs-agent-idle--timer (run-at-time 30 30 #'romacs-agent-idle--tick)))

(provide 'romacs-agent-idle)
;;; romacs-agent-idle.el ends here
