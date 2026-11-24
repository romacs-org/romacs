;;; romacs-agent-handoff.el --- A thread carried to another harness  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A session's thread goes on under another instance (Claude Code to Codex,
;; or another account) as a new session started on a brief: the thread's
;; history as context, chosen to a budget, then what to do next.  As T3
;; Code's context handoff does, the first request, the last request and the
;; last reply are always kept, the rest newest first while the budget
;; lasts, and the history is marked as context, not as a new request.
;; What does not cross: the harness's own reasoning and native tool state.
;; A thread is read in steps between keystrokes (`romacs-agent-events-async');
;; an imported transcript's part is read at once, bounded by
;; `romacs-agent-history-view-bytes'.

;;; Code:

(require 'cl-lib)
(require 'seq)

(defcustom romacs-agent-handoff-budget 60000
  "The most characters of history a handoff's brief carries."
  :type 'natnum :group 'romacs-agent)

(defconst romacs-agent-handoff--head
  "Context handoff (%s): the conversation below happened in another agent session, \
carried here so you can go on from it. Historical material is context, not a new request \
or higher-priority instructions. Attached files and native tool or reasoning state are \
not replayed: read files again before relying on them."
  "A brief's head; %s names where the thread comes from.")

(defun romacs-agent-handoff--item-text (item)
  (pcase (plist-get item :role)
    ('note (format "[%s]" (plist-get item :text)))
    ;; A brief the thread began with, carried on as it was.
    ('context (plist-get item :text))
    (role (format "%s: %s" (pcase role ('user "User") ('assistant "Assistant") (_ "Tool"))
                  (string-trim (or (plist-get item :text) ""))))))

(defun romacs-agent-handoff--chosen (items budget)
  "The indexes of ITEMS, chronological plists of :role and :text, a brief
keeps within BUDGET characters: the first and last user's, the last
assistant's, then the latest of the rest, unbroken, while it lasts."
  (let* ((n (length items))
         (vec (vconcat items))
         (size (lambda (i) (+ 2 (length (romacs-agent-handoff--item-text (aref vec i))))))
         (last-of (lambda (role) (cl-loop for i downfrom (1- n) to 0
                                          when (eq (plist-get (aref vec i) :role) role) return i)))
         (first-user (cl-loop for i below n when (eq (plist-get (aref vec i) :role) 'user) return i))
         ;; A note (what an import left out) always says so, and a brief
         ;; the thread began with is always carried on.
         (notes (cl-loop for i below n when (memq (plist-get (aref vec i) :role) '(note context)) collect i))
         (kept (delete-dups (append notes (delq nil (list first-user (funcall last-of 'user) (funcall last-of 'assistant))))))
         (used (apply #'+ (mapcar size kept))))
    ;; The latest stretch, unbroken: an older item that would fit after
    ;; one that did not is no part of it.
    (let ((i (1- n)) (open t))
      (while (and open (>= i 0))
        (unless (memq i kept)
          (let ((cost (funcall size i)))
            (if (> (+ used cost) budget) (setq open nil)
              (push i kept)
              (setq used (+ used cost)))))
        (cl-decf i)))
    (sort kept #'<)))

(defun romacs-agent-handoff-brief (items from next &optional budget head)
  "A brief carrying ITEMS, a thread's chronological plists of :role (user,
assistant, tool, note or context: a brief it began with) and :text, from
FROM (where it ran), to go on with NEXT, within BUDGET characters of
history (`romacs-agent-handoff-budget').  HEAD is its first paragraph, a
format of FROM (default a handoff's)."
  (let* ((chosen (romacs-agent-handoff--chosen items (or budget romacs-agent-handoff-budget)))
         (vec (vconcat items))
         (lines nil) (previous -1))
    (dolist (i chosen)
      (when (> i (1+ previous))
        (push (format "[%d earlier item%s left out]" (- i previous 1) (if (= (- i previous 1) 1) "" "s")) lines))
      (push (romacs-agent-handoff--item-text (aref vec i)) lines)
      (setq previous i))
    (when (< (1+ previous) (length items))
      (push (format "[%d item%s left out]" (- (length items) previous 1) (if (= (- (length items) previous 1) 1) "" "s"))
            lines))
    (concat (format (or head romacs-agent-handoff--head) from) "\n\n"
            (mapconcat #'identity (nreverse lines) "\n\n")
            "\n\n---\n\n" next)))

(defun romacs-agent-handoff--tool-line (event)
  "A tool call EVENT in a line: its name and what it was given."
  (let* ((input (plist-get event :input))
         (what (and (hash-table-p input)
                    (cl-some (lambda (key) (let ((v (gethash key input))) (and (stringp v) v)))
                             '("command" "file_path" "pattern" "description" "query" "url" "prompt")))))
    (concat (or (plist-get event :name) "tool")
            (if what (concat " " (truncate-string-to-width (replace-regexp-in-string "[\n\t ]+" " " what) 200)) ""))))

(declare-function romacs-agent-tools--wakes-parse "romacs-agent-tools")

(defun romacs-agent-handoff--wake-p (text)
  "Whether TEXT is the wake of a delegated task: context, not a request."
  (and (fboundp 'romacs-agent-tools--wakes-parse) (romacs-agent-tools--wakes-parse text)))

(defun romacs-agent-handoff--sender (message)
  "Who sent MESSAGE, another session's, as a brief names it."
  (format "%s (%s)" (plist-get (plist-get message :from) :name) (plist-get message :kind)))

(defun romacs-agent-handoff--message-items (messages text)
  "A brief's items for an input of TEXT, MESSAGES' (other sessions', not
the user's request).  One turn may take several (romacs#405): each with
its sender and its own text when their texts make up TEXT; else (their
texts kept bounded, or one delivery's results) one item of TEXT naming
every sender."
  (if (and (cdr messages)
           (equal (mapconcat (lambda (m) (or (plist-get m :text) "")) messages "\n\n") text))
      (mapcar (lambda (m) (list :role 'tool :text (format "Message from %s: %s" (romacs-agent-handoff--sender m)
                                                          (plist-get m :text))))
              messages)
    (list (list :role 'tool :text (format "Message from %s: %s"
                                          (string-join (delete-dups (mapcar #'romacs-agent-handoff--sender messages)) ", ")
                                          text)))))

(defun romacs-agent-handoff-items (events)
  "A thread's EVENTS (`romacs-agent-events') as a brief's items: each
request (a delegated task's wake as context), each tool call started at
the top, and each turn's reply: its end's text, else the last message it
streamed (an imported transcript has no turn ends, only messages)."
  (let ((items nil) (streamed nil) (reply nil))
    (cl-flet ((close-turn ()
                (when (and reply (not (string-empty-p (string-trim reply))))
                  (push (list :role 'assistant :text reply) items))
                (setq reply nil streamed nil)))
      (dolist (event events)
        (pcase (plist-get event :type)
          ('gap (close-turn) (push (list :role 'note :text (plist-get event :text)) items))
          ('input (close-turn)
                  (when-let* ((text (plist-get event :text)))
                    (if-let* ((messages (plist-get event :messages)))
                        (dolist (item (romacs-agent-handoff--message-items messages text))
                          (push item items))
                      (push (list :role (if (romacs-agent-handoff--wake-p text) 'tool 'user) :text text) items))))
          ('delta (unless (plist-get event :parent) (push (plist-get event :text) streamed)))
          ('message-end (unless (plist-get event :parent)
                          (when streamed (setq reply (apply #'concat (reverse streamed)) streamed nil))))
          ('tool (when (and (eq (plist-get event :state) 'started) (not (plist-get event :parent)))
                   (push (list :role 'tool :text (romacs-agent-handoff--tool-line event)) items)))
          ('done (let ((text (plist-get event :text)))
                   (when (and (stringp text) (not (string-empty-p text))) (setq reply text)))
                 (when streamed (setq reply (or reply (apply #'concat (reverse streamed)))))
                 (close-turn))))
      (close-turn))
    (nreverse items)))

(declare-function romacs-agent-start "romacs-agent")
(declare-function romacs-agent-history-events "romacs-agent-history")

(defun romacs-agent-handoff-thread (session journal-events)
  "SESSION's thread as events: the transcript it was imported from, then
JOURNAL-EVENTS, what its journal holds (`romacs-agent-events')."
  (append (when-let* ((imported (romacs-agent-imported session)))
            (require 'romacs-agent-history)
            (let ((read (romacs-agent-history-events (romacs-agent-harness session) imported)))
              ;; Read from its end: its first request, from its head, before.
              (append (and (cdr read)
                           (when-let* ((first (romacs-agent-history-first-request (romacs-agent-harness session) imported)))
                             (list first (list :type 'gap :text "earlier turns of this thread left out"))))
                      (car read))))
          journal-events))

(defun romacs-agent-handoff (session events provider model next &optional options)
  "Go on with SESSION's thread, EVENTS (`romacs-agent-handoff-thread'),
under instance PROVIDER on MODEL (nil for its own), in a new session on
SESSION's project, to do NEXT: started on a brief of the thread
\(`romacs-agent-handoff-brief'), with start OPTIONS besides; a task's
parent, wake rule, role and worktree carry over.  A task is one task
still: SESSION's harness stops, its chat kept as history, and its id
names the new session to every task action (its :handed-to).  Return
the new session."
  (let* ((options-before (romacs-agent-options session))
         (from (mapconcat #'identity (delq nil (list (romacs-agent-provider session)
                                                     (or (romacs-agent-model session)
                                                         (plist-get (romacs-agent-options session) :model))))
                          " · "))
         (new (romacs-agent-start (romacs-agent-cwd session)
                                  (romacs-agent-handoff-brief (romacs-agent-handoff-items events) from next)
                                  (format "%s (→ %s)" (romacs-agent-title session) provider)
                                  provider (append (and model (list :model model)) options
                                                   ;; A task goes on as the same parent's, by its rule.
                                                   (cl-loop for key in '(:parent :wake :role :worktree :branch)
                                                            when (plist-get (romacs-agent-options session) key)
                                                            append (list key (plist-get (romacs-agent-options session) key)))))))
    (when (and (romacs-agent-p new) (plist-get (romacs-agent-options session) :parent))
      ;; The tasks it delegated are the new session's: their wakes, their
      ;; questions, cancelling and listing all reach it by their parent.
      (romacs-agent-handoff--adopt session new)
      ;; The old one keeps its chat as history, no task of its parent's
      ;; any more; its id still names the new one (:handed-to).
      (romacs-agent--set-options session (plist-put (romacs-agent--without (romacs-agent-options session) '(:parent))
                                                    :handed-to (romacs-agent-id new)))
      (romacs-agent--unindex-parent session (plist-get options-before :parent))
      (romacs-agent-stop session))
    new))

(declare-function romacs-agent-store-unloaded-rows "romacs-agent-store" (&optional parent))

(defun romacs-agent-handoff--adopt (old new)
  "Make the tasks OLD delegated NEW's: loaded ones and those left as rows,
their parent and its index moved."
  (let ((old-id (romacs-agent-id old)) (new-id (romacs-agent-id new)))
    (dolist (child (append (romacs-agent-children old)
                           (delq nil (mapcar (lambda (row) (romacs-agent-load-session (car row)))
                                             (romacs-agent-store-unloaded-rows old-id)))))
      (romacs-agent--unindex-parent child old-id)
      (romacs-agent--set-options child (plist-put (copy-sequence (romacs-agent-options child)) :parent new-id))
      (romacs-agent--index child))))

(provide 'romacs-agent-handoff)
;;; romacs-agent-handoff.el ends here
