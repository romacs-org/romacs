;;; romacs-agent-menu.el --- An agent chat's actions menu  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The actions of an agent chat (romacs-agent-log.el), as a keymap-popup
;; menu: what waits on the user, the turn, what the session runs on and
;; the workers it delegated that are at work, the session, the view.  Its
;; title names the session; each worker row says its state, then its
;; title, and its digit opens the worker the row showed.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent)
(require 'keymap-popup)
(require 'romacs-working)
(require 'romacs-agent-log)

(declare-function romacs-agents "romacs-agent-board" ())
(declare-function romacs-agent-resume-time "romacs-agent-tools" (session))
(declare-function romacs-agent-tools--cancel-resume "romacs-agent-tools" (session &optional why))
(declare-function romacs-agent-ui--batch "romacs-agent-ui" ())
(declare-function romacs-agent-ui--selection-label "romacs-agent-ui" (selection))
(declare-function romacs-agent-ui--instance "romacs-agent-ui" (provider harness model default))

(defun romacs-agent-menu--title ()
  "The actions menu's title: whose chat it is, its name first to read."
  (concat "Agent · " (propertize (or (and romacs-agent-log--session (romacs-agent-title romacs-agent-log--session)) "chat")
                                 'face 'bold)))

(defun romacs-agent-menu--workers ()
  "The sessions this chat's delegated that are at work, those waiting on
the user first: a finished one is the board's."
  (when romacs-agent-log--session
    (let ((live (seq-filter (lambda (child) (memq (romacs-agent-state child) '(starting running waiting-approval)))
                            (romacs-agent-children romacs-agent-log--session))))
      (append (seq-filter (lambda (child) (eq (romacs-agent-state child) 'waiting-approval)) live)
              (seq-remove (lambda (child) (eq (romacs-agent-state child) 'waiting-approval)) live)))))

(defvar-local romacs-agent-menu--shown nil
  "The worker each digit's row last showed, however the menu was opened,
a vector of nine, the chat's own: a digit opens that one, saying so when
it finished since.")

(defun romacs-agent-menu--slots ()
  (unless (and (local-variable-p 'romacs-agent-menu--shown)
               (vectorp romacs-agent-menu--shown) (= (length romacs-agent-menu--shown) 9))
    (setq romacs-agent-menu--shown (make-vector 9 nil)))
  romacs-agent-menu--shown)

(defun romacs-agent-menu--worker-state (child)
  "CHILD's state in the menu: the board's mark and a word, in its face."
  (if (eq (romacs-agent-state child) 'waiting-approval)
      (cons (propertize "◆" 'face 'romacs-working-waiting) (propertize "asks you" 'face '(romacs-working-waiting bold)))
    (cons (propertize "●" 'face 'romacs-working-running) (propertize "running" 'face '(romacs-working-running bold)))))

(defun romacs-agent-menu--worker-label (index)
  "The menu's row of the worker at INDEX: a mark and a word for what it
does, then its title, so a long title is what the menu cuts; nil, no
row, when there is none.  The row's digit then opens what it showed."
  (when-let* ((child (aset (romacs-agent-menu--slots) index (nth index (romacs-agent-menu--workers)))))
    ;; Its state first: whatever cuts the row, at any width, cuts the title.
    (let ((state (romacs-agent-menu--worker-state child)))
      (concat (car state) " " (cdr state) " · " (romacs-agent-title child)))))

(defun romacs-agent-menu--open-worker (child)
  "Open worker CHILD's chat, unless it finished since it was shown."
  (unless (memq child (romacs-agent-menu--workers))
    (user-error "%s is no longer at work: the board has it" (romacs-agent-title child)))
  (romacs-agent-show child '(display-buffer-same-window)))

(defun romacs-agent-menu-open-worker ()
  "Open the chat of the worker the menu showed on the digit just pressed."
  (interactive nil romacs-agent-log-mode)
  (let ((index (- last-command-event ?1)))
    (romacs-agent-menu--open-worker (or (and (< -1 index 9) (aref (romacs-agent-menu--slots) index))
                                       (user-error "No such worker")))))

(defun romacs-agent-menu-workers ()
  "Open the chat of one of the workers at work, chosen by its title."
  (interactive nil romacs-agent-log-mode)
  (let* ((workers (or (romacs-agent-menu--workers) (user-error "No worker is at work")))
         ;; Its id makes each line its own, whatever the titles.
         (named (mapcar (lambda (child) (cons (format "%s  %s" (romacs-agent-title child) (romacs-agent-id child)) child))
                        workers)))
    (romacs-agent-menu--open-worker
     (cdr (assoc (completing-read "Worker: " (romacs-agent--ordered-table (mapcar #'car named)) nil t) named)))))

;;; The session's commands, as the menu offers them

(defun romacs-agent-chat-stop ()
  "Stop the session."
  (interactive nil romacs-agent-log-mode)
  (when (yes-or-no-p "Stop this agent session? ") (romacs-agent-stop (romacs-agent-log--loaded))))

(defun romacs-agent-chat-resume () "Resume the session." (interactive nil romacs-agent-log-mode) (romacs-agent-resume (romacs-agent-log--loaded)))

(defun romacs-agent-chat-fork ()
  "Go on from this session's conversation in a new session, its own chat;
this one goes on too (`romacs-agent-fork')."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-show (romacs-agent-fork (romacs-agent-log--loaded))))

(defun romacs-agent-chat-renew ()
  "Go on in a new conversation of this session's harness, its next message
carrying a brief of this one (`romacs-agent-renew'): its context is freed,
the session stays."
  (interactive nil romacs-agent-log-mode)
  (let ((session (romacs-agent-log--loaded)))
    (romacs-agent-renew session "by hand"
                        (lambda (outcome)
                          (run-at-time 0 nil (lambda ()
                                               (pcase outcome
                                                 (`(:done ,_) (message "Renewed: a new conversation"))
                                                 (`(:error ,why) (message "Not renewed: %s" why)))))))))

(defun romacs-agent-chat-rename (title)
  "Name this session TITLE."
  (interactive (list (read-string "Title: " (romacs-agent-title (romacs-agent-log--loaded))))
               romacs-agent-log-mode)
  (let ((session (romacs-agent-log--loaded)))
    (setf (romacs-agent-title session) title (romacs-agent-title-auto session) nil)
    ;; Its views drawn again now, and once its row says it: those of a
    ;; session not loaded read the row.
    (romacs-agent--update-manifest session #'romacs-agent-ui--batch)
    (rename-buffer (romacs-agent-log--buffer-name session) t)
    (romacs-agent--changed session)
    (romacs-agent-ui--batch)))

(defun romacs-agent-chat-effort (effort)
  "Run this session at reasoning EFFORT from its next turn on, its model
kept; nil leaves it to the harness."
  (interactive
   (let* ((session (romacs-agent-log--loaded))
          (efforts (or (romacs-agent--efforts session) (user-error "This model offers no effort"))))
     (list (romacs-agent--read-effort efforts (plist-get (romacs-agent--wanted session) :effort))))
   romacs-agent-log-mode)
  (let ((session (romacs-agent-log--loaded)))
    (unless (romacs-agent-select session (plist-get (romacs-agent--wanted session) :model) effort)
      (message "Effort is already %s" (or effort "the harness's")))))

(defun romacs-agent-chat-model (model effort)
  "Run this session on MODEL, of its own instance, from its next turn on,
at EFFORT: the effort it runs at when MODEL takes it, else MODEL's default;
with a prefix argument, asked for."
  (interactive
   (let* ((session (romacs-agent-log--loaded))
          (instance (romacs-agent--instance (romacs-agent-provider session) (romacs-agent-harness session)))
          (fields (cdr (romacs-agent--pick-row
                        (romacs-agent--instance-rows instance (romacs-agent--harness-display (romacs-agent-harness session)) nil))))
          (model (plist-get fields :model))
          (efforts (plist-get fields :efforts))
          (now (plist-get (romacs-agent--wanted session) :effort))
          (fitting (if (member now efforts) now (plist-get fields :default-effort))))
     (list model (if (and current-prefix-arg efforts) (romacs-agent--read-effort efforts fitting) fitting)))
   romacs-agent-log-mode)
  (unless (romacs-agent-select (romacs-agent-log--loaded) model effort)
    (message "Already on %s" (romacs-agent-ui--selection-label (list :model model :effort effort)))))

(defun romacs-agent-menu--model-label ()
  "The model entry: the model running, and the one to come if it differs."
  (let* ((session romacs-agent-log--session)
         (harness (and session (romacs-agent-harness session)))
         (shown (lambda (model)
                  (propertize (or (and model (romacs-agent-ui--instance nil harness model nil)) "default")
                              ;; A menu's value is never dim: dim is a command it cannot run.
                              'face 'bold)))
         (now (and session (or (plist-get (romacs-agent--selection session) :model) (romacs-agent-model session))))
         (next (and session (plist-get (romacs-agent--wanted session) :model))))
    (concat "Model: " (funcall shown now)
            (if (or (null (romacs-agent-pending-selection session)) (equal now next)) ""
              (concat " → " (funcall shown next))))))

(defun romacs-agent-chat-handoff (provider model next)
  "Go on with this thread under another instance or model: PROVIDER and
MODEL picked as for a new session.  Under an instance of the same harness
\(claude to claude2) this session goes on, its conversation moved as is,
once a running turn ends.  Under another harness a new session starts on
a brief of the thread, to do NEXT, and its chat opens."
  (interactive
   (let* ((fields (cdr (romacs-agent--pick-row (romacs-agent--picker-rows))))
          (provider (plist-get fields :provider)))
     (list provider (plist-get fields :model)
           (unless (romacs-agent-menu--moves-p (romacs-agent-log--loaded) provider)
             (read-string "Go on with: " nil nil "Go on from where this thread left off."))))
   romacs-agent-log-mode)
  (let ((session (romacs-agent-log--loaded)))
    (if (and (symbolp provider) (romacs-agent-menu--moves-p session provider))
        (romacs-agent-chat-next-account)
      (if (romacs-agent-menu--moves-p session provider)
        (let ((wanted (romacs-agent--wanted session)))
          (romacs-agent-select session (or model (plist-get wanted :model)) (plist-get wanted :effort) provider)
          (message "%s goes on under %s, its conversation as is%s" (romacs-agent-title session) provider
                   (if (memq (romacs-agent-state session) '(starting running waiting-approval)) ", once this turn ends" "")))
      (romacs-agent-menu--hand-off session provider model next)))))

(defun romacs-agent-chat-next-account ()
  "Go on under the next account of this session's harness with room, its
conversation as is (`romacs-agent-move-on')."
  (interactive nil romacs-agent-log-mode)
  (let* ((session (romacs-agent-log--loaded)) (to (romacs-agent-move-on session)))
    (message "%s goes on under %s, its conversation as is%s" (romacs-agent-title session) to
             (if (memq (romacs-agent-state session) '(starting running waiting-approval)) ", once this turn ends" ""))))

(defun romacs-agent-menu--moves-p (session provider)
  "Whether SESSION goes on under PROVIDER as it is: its harness's pool, or
an instance of its harness."
  (eq (if (symbolp provider) provider (plist-get (romacs-agent--configured provider) :harness))
      (romacs-agent-harness session)))

(defun romacs-agent-menu--hand-off (session provider model next)
  "Start a session under PROVIDER on MODEL on a brief of SESSION's thread,
to do NEXT."
  (message "Gathering %s's thread..." (romacs-agent-title session))
    ;; Replayed between keystrokes: a long thread never holds the editor.
    (romacs-agent-events-async
     session (lambda (events)
               (romacs-agent-show (romacs-agent-handoff session (romacs-agent-handoff-thread session events)
                                                        provider model next)))))

(defun romacs-agent-chat-plan ()
  "Put this session in plan mode, or take it out: in plan mode it plans
and changes nothing; its plan comes as a card to implement."
  (interactive nil romacs-agent-log-mode)
  (let ((session (romacs-agent-log--loaded)))
    (romacs-agent-set-plan session (not (plist-get (romacs-agent-options session) :plan)))))

(defun romacs-agent-menu--plan-label ()
  (if (and romacs-agent-log--session (plist-get (romacs-agent-options romacs-agent-log--session) :plan))
      (concat "Plan mode: " (propertize "on" 'face 'bold))
    (concat "Plan mode: " (propertize "off" 'face 'bold))))

(defun romacs-agent-chat-new ()
  "Start a new session, in this one's directory."
  (interactive nil romacs-agent-log-mode)
  (apply #'funcall-interactively #'romacs-agent-start
         (romacs-agent--start-arguments (file-name-as-directory (romacs-agent-cwd (romacs-agent-log--loaded))))))

(defun romacs-agent-menu--effort-label ()
  "The effort entry: the effort running, and the one to come if it differs."
  (let* ((session romacs-agent-log--session)
         (now (and session (plist-get (romacs-agent--selection session) :effort)))
         (next (and session (plist-get (romacs-agent--wanted session) :effort))))
    ;; A menu's value is never dim: dim is a command it cannot run.
    (cl-flet ((shown (effort) (propertize (or effort "default") 'face 'bold)))
      (concat "Effort: " (shown now) (if (equal now next) "" (concat " → " (shown next)))))))

;;; What the chat does with its turns and files

(defun romacs-agent-menu--last-prompt (session)
  "The request id of SESSION's last prompt still in its conversation."
  (car (last (delq nil (mapcar #'romacs-agent--record-request
                               (romacs-agent--live-records (romacs-agent--journal-records session)))))))

(defun romacs-agent-chat-rollback (&optional last)
  "Take this session back to just before the turn at point (with a prefix
argument LAST, its last turn): its conversation, and its files when the
turn has a checkpoint.  Without one, offer the conversation alone.  Files
changed meanwhile by something else are named, and restored only when
you say so."
  (interactive "P" romacs-agent-log-mode)
  (let* ((session (romacs-agent-log--loaded))
         (request (or (cond (last (romacs-agent-menu--last-prompt session))
                            ;; From the composer, only the last turn, asked for.
                            ((>= (point) romacs-agent-log--end)
                             (user-error (substitute-command-keys
                                          "Put point on a turn, or \\[universal-argument] for the last")))
                            (t (romacs-agent-log--turn-here)))
                      (user-error "No turn here")))
         (files (seq-find (lambda (c) (and (equal (car c) request) (eq (nth 1 c) 'start)))
                          (romacs-agent-diff--checkpoints session))))
    (unless (let ((turn (truncate-string-to-width (or (romacs-agent-log--turn-text request) "the last turn") 50 nil nil t)))
              (if files (y-or-n-p (format "Roll back to before %s, its conversation and its files? " turn))
                (y-or-n-p (format "No checkpoint of %s's files: roll back its conversation only? " turn))))
      (user-error "Not rolled back"))
    (cl-labels ((go (force)
                  (romacs-agent-rollback
                   session request
                   (lambda (outcome)
                     ;; Asked from the command loop, not from git's sentinel.
                     (run-at-time 0 nil
                                  (lambda ()
                                    (pcase outcome
                                      (`(:done ,n) (message "Rolled back %d turn%s" n (if (= n 1) "" "s")))
                                      (`(:clobbered ,names)
                                       (if (yes-or-no-p (format "Changed meanwhile by something else: %s.  Restore them anyway? "
                                                                (string-join names ", ")))
                                           (go t)
                                         (message "Not rolled back")))
                                      (`(:error ,why) (message "Not rolled back: %s" why))))))
                   :files (and files t) :force force)))
      (go nil))))

(defun romacs-agent-chat-diff (&optional all)
  "Show what the turn at point changed in the project (the last turn
from the composer) in `diff-mode', in another window; with a prefix
argument ALL, what every turn of the session did."
  (interactive "P" romacs-agent-log-mode)
  (let ((session (romacs-agent-log--loaded)))
    (romacs-agent-diff-turn session (unless all (or (and (< (point) romacs-agent-log--end) (romacs-agent-log--turn-here))
                                                    (romacs-agent-diff-current session)
                                                    (user-error "No turn yet"))))))

(defun romacs-agent-chat-cancel-resume ()
  "This session no longer resumes by itself when its limit resets."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-tools--cancel-resume (romacs-agent-log--loaded) "cancelled by hand"))

(defun romacs-agent-chat-attach (file)
  "Put FILE's path in the composer: the agent reads it where it is, no
copy is made."
  (interactive (list (read-file-name "Attach: " nil nil t)) romacs-agent-log-mode)
  (goto-char (point-max))
  (unless (or (= (point) romacs-agent-log--input) (memq (char-before) '(?\s ?\n))) (insert " "))
  (insert (expand-file-name file)))

(keymap-popup-define romacs-agent-menu-map
  "Actions of an agent chat: what waits on you, the turn, what it runs on,
the session, the view."
  :popup-key "?"
  :exit-key "C-g"
  :description #'romacs-agent-menu--title
  :group ("Waits on you" :if (lambda () (and romacs-agent-log--session (romacs-agent-log--pending))))
  "y" ("Allow" romacs-agent-log-allow)
  "!" ("Allow for this session" romacs-agent-log-allow-always)
  "d" ("Deny" romacs-agent-log-deny)
  "j" ("Go to it" romacs-agent-log-next-ask)
  :group "Turn"
  "i" ("Interrupt" romacs-agent-log-interrupt :inapt-if (lambda () (not (romacs-agent-log--running-p))))
  "k" ("Interrupt, then send" romacs-agent-log-interrupt-and-send)
  "q" ("Send when the turn ends" romacs-agent-log-queue)
  "u" ("Roll back to before this turn (C-u: the last)" romacs-agent-chat-rollback
       :inapt-if (lambda () (not (romacs-agent--harness-free-p romacs-agent-log--session))))
  "D" ("Diff of this turn (C-u: all)" romacs-agent-chat-diff)
  :group "Runs on"
  "m" (#'romacs-agent-menu--model-label romacs-agent-chat-model)
  "E" (#'romacs-agent-menu--effort-label romacs-agent-chat-effort)
  "P" (#'romacs-agent-menu--plan-label romacs-agent-chat-plan)
  "H" ("Go on under another instance" romacs-agent-chat-handoff)
  "N" ("Next account with room" romacs-agent-chat-next-account)
  "z" ("Cancel the resume at its limit's reset" romacs-agent-chat-cancel-resume
       :if (lambda () (and romacs-agent-log--session (romacs-agent-resume-time romacs-agent-log--session))))
  ;; No :if: a group whose rows are all empty is not drawn, and a digit
  ;; whose worker finished since says so rather than falling through.
  :group "Workers"
  "1" ((lambda () (romacs-agent-menu--worker-label 0)) romacs-agent-menu-open-worker)
  "2" ((lambda () (romacs-agent-menu--worker-label 1)) romacs-agent-menu-open-worker)
  "3" ((lambda () (romacs-agent-menu--worker-label 2)) romacs-agent-menu-open-worker)
  "4" ((lambda () (romacs-agent-menu--worker-label 3)) romacs-agent-menu-open-worker)
  "5" ((lambda () (romacs-agent-menu--worker-label 4)) romacs-agent-menu-open-worker)
  "6" ((lambda () (romacs-agent-menu--worker-label 5)) romacs-agent-menu-open-worker)
  "7" ((lambda () (romacs-agent-menu--worker-label 6)) romacs-agent-menu-open-worker)
  "8" ((lambda () (romacs-agent-menu--worker-label 7)) romacs-agent-menu-open-worker)
  "9" ((lambda () (romacs-agent-menu--worker-label 8)) romacs-agent-menu-open-worker)
  "o" ((lambda () (let ((n (length (romacs-agent-menu--workers))))
                    (and (> n 9) (format "All %d at work" n))))
       romacs-agent-menu-workers)
  :row
  :group "Session"
  "b" ("Agent board" romacs-agents)
  "n" ("New session" romacs-agent-chat-new)
  "R" ("Rename" romacs-agent-chat-rename)
  "f" ("Fork" romacs-agent-chat-fork)
  "F" ("Fresh conversation, briefed" romacs-agent-chat-renew)
  "r" ("Resume" romacs-agent-chat-resume :inapt-if #'romacs-agent-log--running-p)
  "x" ("Stop" romacs-agent-chat-stop)
  :group "View"
  "e" ("Earlier turns" romacs-agent-log-earlier)
  "w" ("Copy the entry at point" romacs-agent-log-copy)
  "A" ("Attach a file (its path)" romacs-agent-chat-attach))

(put 'romacs-agent-menu-map-popup 'command-modes '(romacs-agent-log-mode))

(provide 'romacs-agent-menu)
;;; romacs-agent-menu.el ends here
