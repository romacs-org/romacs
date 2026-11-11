;;; romacs-agent-idle.el --- Compact an idle thread while its cache is warm  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A large idle Claude Code thread is compacted just before its prompt
;; cache expires, as T3 Code does (IdleCompaction.ts): the compaction reads
;; the context at the cached price, and the next prompt starts from the
;; summary instead of paying a cold read of the whole context.  Only a
;; thread worth it (at least 200k tokens of context, Claude Code's own
;; idle-compaction floor), whose turn ended, with nothing waiting on it or
;; running under it, once per idle spell, never a delegated task's.

;;; Code:

(require 'romacs-agent)

(defcustom romacs-agent-idle-compaction t
  "Whether a large idle Claude Code thread is compacted just before its
prompt cache expires: its last request's time and the cache's life that
request reported (an hour, or five minutes)."
  :type 'boolean :group 'romacs-agent)

(defconst romacs-agent-idle--min-tokens 200000
  "The least context worth compacting: below it a cold read is cheap.")

(defconst romacs-agent-idle--lead 120
  "Seconds before the cache expires to compact: time for the compaction
to start while it is still warm.")

(defun romacs-agent-idle--busy-p (session)
  "Whether anything waits on SESSION or runs under it: an approval, a
message queued, a selection waiting, its harness's own subagents or
commands, or a task it delegated."
  (or (> (hash-table-count (romacs-agent-approvals session)) 0)
      (romacs-agent-queue session)
      (romacs-agent-pending-selection session)
      (and (romacs-agent-subagents session)
           (cl-some (lambda (row) (equal (plist-get row :state) "running"))
                    (hash-table-values (romacs-agent-subagents session))))
      (cl-some (lambda (child) (memq (romacs-agent-state child) '(starting running waiting-approval)))
               (romacs-agent-children session))))

(defun romacs-agent-idle-due-p (session now)
  "Whether SESSION's thread is to be compacted at NOW (`float-time')."
  (let ((expires (plist-get (romacs-agent-driver-state session) :cache-expires)))
    (and romacs-agent-idle-compaction
         (eq (romacs-agent-harness session) 'claude)
         (not (plist-get (romacs-agent-options session) :parent))
         (eq (romacs-agent-get (romacs-agent-id session)) session)
         (romacs-agent-caught-up session)
         (process-live-p (romacs-agent-connection session))
         (romacs-agent--ready-p session)
         (eq (romacs-agent-state session) 'idle)
         (not (romacs-agent-failed session))
         (>= (or (romacs-agent-context-used session) 0) romacs-agent-idle--min-tokens)
         ;; Never a compaction's compaction: once per idle spell.
         (not (plist-get (romacs-agent-driver-state session) :compact-turn))
         ;; Unknown (no request this run, or one served wholly from the
         ;; cache): not guessed.
         (numberp expires)
         (>= now (- expires romacs-agent-idle--lead)) (< now expires)
         (not (romacs-agent-idle--busy-p session)))))

(defun romacs-agent-idle--compact (session)
  "Compact SESSION's thread: Claude Code's own /compact, as a message."
  (romacs-agent-send session "/compact"))

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
