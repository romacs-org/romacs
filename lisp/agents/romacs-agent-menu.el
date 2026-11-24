;;; romacs-agent-menu.el --- An agent chat's actions menu  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The actions of an agent chat (romacs-agent-log.el), as a keymap-popup
;; menu: what waits on the user, the turn, what the session runs on and
;; the unsettled workers it delegated, the session, the view.  Its
;; title names the session; each worker row says its state, then its
;; title, and its digit opens the worker the row showed.

;;; Code:

(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent)
(require 'romacs-agent-glyphs)
(require 'keymap-popup)
(require 'romacs-working)
(require 'romacs-agent-log)
(require 'romacs-attach)

(defvar romacs-complete-file-preview)

(declare-function romacs-agents "romacs-agent-board" ())
(declare-function romacs-agent-resume-time "romacs-agent-tools" (session))
(declare-function romacs-agent-tools--cancel-resume "romacs-agent-tools" (session &optional why))
(declare-function romacs-agent-ui--batch "romacs-agent-ui" ())
(declare-function romacs-agent-ui--default-effort "romacs-agent-ui" (session selection))
(declare-function romacs-agent-ui--selection-label "romacs-agent-ui" (selection))
(declare-function romacs-agent-ui--instance "romacs-agent-ui" (provider harness model default))
(declare-function romacs-agent-log-attach-paste "romacs-agent-log-attach" (type at &optional spaced))
(declare-function romacs-agent-compose-transcript "romacs-agent-log-compose" ())

(defun romacs-agent-menu--title ()
  "The actions menu's title: whose chat it is, its name first to read."
  (concat "Agent · " (propertize (or (and romacs-agent-log--session (romacs-agent-title romacs-agent-log--session)) "chat")
                                 'face 'bold)))

(defun romacs-agent-menu--worker-rows ()
  "The chat's unsettled delegated rows, in panel order."
  (when romacs-agent-log--session
    (pcase-let ((`(,live . ,inactive) (romacs-agent-log-activity-delegated-rows
                                        (romacs-agent-children-rows romacs-agent-log--session))))
      (append live inactive))))

(defun romacs-agent-menu--workers ()
  "The chat's unsettled delegated sessions, in panel order."
  (mapcar (lambda (row) (plist-get row :session)) (romacs-agent-menu--worker-rows)))

(defvar-local romacs-agent-menu--shown nil
  "The worker each digit's row last showed, however the menu was opened,
a vector of nine, the chat's own: a digit opens that one, saying so when
it settled since.")

(defun romacs-agent-menu--slots ()
  (unless (and (local-variable-p 'romacs-agent-menu--shown)
               (vectorp romacs-agent-menu--shown) (= (length romacs-agent-menu--shown) 9))
    (setq romacs-agent-menu--shown (make-vector 9 nil)))
  romacs-agent-menu--shown)

(defun romacs-agent-menu--worker-label (index)
  "Draw the panel row for the worker at INDEX and remember its digit."
  (let* ((row (nth index (romacs-agent-menu--worker-rows)))
         (child (aset (romacs-agent-menu--slots) index (plist-get row :session))))
    (when child
      (propertize (romacs-agent-log-activity-delegated-line row keymap-popup--max-description-width)
                  'keymap-popup-preformatted t))))

(defun romacs-agent-menu--open-worker (child)
  "Open worker CHILD's chat if it is still among the unsettled workers."
  (if-let* ((current (seq-find (lambda (worker) (equal (romacs-agent-id worker) (romacs-agent-id child)))
                              (romacs-agent-menu--workers)))
              ((not (romacs-agent-settled-p current))))
      (romacs-agent-show current '(display-buffer-same-window))
    (user-error "%s is no longer in the Workers list" (romacs-agent-title child))))

(defun romacs-agent-menu-open-worker ()
  "Open the chat of the worker the menu showed on the digit just pressed."
  (interactive nil romacs-agent-log-mode)
  (let ((index (- last-command-event ?1)))
    (romacs-agent-menu--open-worker (or (and (< -1 index 9) (aref (romacs-agent-menu--slots) index))
                                       (user-error "No such worker")))))

(defun romacs-agent-menu-workers ()
  "Open the chat of an unsettled delegated worker, chosen by its title."
  (interactive nil romacs-agent-log-mode)
  (let* ((workers (or (romacs-agent-menu--workers) (user-error "No unsettled delegated workers")))
         ;; Its id makes each line its own, whatever the titles.
         (named (mapcar (lambda (child) (cons (format "%s  %s" (romacs-agent-title child) (romacs-agent-id child)) child))
                        workers)))
    (romacs-agent-menu--open-worker
     (cdr (assoc (completing-read "Worker: " (romacs-agent--ordered-table (mapcar #'car named)) nil t) named)))))

;;; The session's commands, as the menu offers them

(defun romacs-agent-menu--session ()
  "The current chat's session, loaded, for a command of this menu, from
the chat or its composer: a chat whose source takes this editor's
commands only (`romacs-agent-log--commands-p')."
  (unless (romacs-agent-log--commands-p) (user-error "This chat's session takes no commands from here"))
  (with-current-buffer (romacs-agent-log--chat)
    (romacs-agent-log--loaded romacs-agent-log--session)))

(defun romacs-agent-chat-stop ()
  "Stop the session."
  (interactive nil romacs-agent-log-mode)
  (when (yes-or-no-p "Stop this agent session? ") (romacs-agent-stop (romacs-agent-menu--session))))

(defun romacs-agent-chat-resume () "Resume the session." (interactive nil romacs-agent-log-mode) (romacs-agent-resume (romacs-agent-menu--session)))

(defun romacs-agent-chat-fork ()
  "Go on from this session's conversation in a new session, its own chat;
this one goes on too (`romacs-agent-fork')."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-show (romacs-agent-fork (romacs-agent-menu--session))))

(defun romacs-agent-chat-renew ()
  "Go on in a new conversation of this session's harness, its next message
carrying a brief of this one (`romacs-agent-renew'): its context is freed,
the session stays."
  (interactive nil romacs-agent-log-mode)
  (let ((session (romacs-agent-menu--session)))
    (romacs-agent-renew session "by hand"
                        (lambda (outcome)
                          (run-at-time 0 nil (lambda ()
                                               (pcase outcome
                                                 (`(:done ,_) (message "Renewed: a new conversation"))
                                                 (`(:error ,why) (message "Not renewed: %s" why)))))))))

(defun romacs-agent-chat-rename (title)
  "Name this session TITLE."
  (interactive (list (read-string "Title: " (romacs-agent-title (romacs-agent-menu--session))))
               romacs-agent-log-mode romacs-agent-compose-mode)
  (romacs-agent-log-in-chat
    (let ((session (romacs-agent-menu--session)))
      (setf (romacs-agent-title session) title (romacs-agent-title-auto session) nil)
      ;; Its views drawn again now, and once its row says it: those of a
      ;; session not loaded read the row.
      (romacs-agent--update-manifest session #'romacs-agent-ui--batch)
      (rename-buffer (romacs-agent-log--buffer-name session) t)
      (romacs-agent--changed session)
      (romacs-agent-ui--batch))))

(defun romacs-agent-chat-effort (effort)
  "Run this session at reasoning EFFORT from its next turn on, its model
kept; nil leaves it to the harness."
  (interactive
   (let* ((session (romacs-agent-menu--session))
          (_ (romacs-agent--want-models
              (list (romacs-agent--instance (romacs-agent-provider session) (romacs-agent-harness session)))))
          (efforts (or (romacs-agent--efforts session) (user-error "This model offers no effort")))
          (wanted (romacs-agent--wanted session)))
     (list (romacs-agent--read-effort efforts (plist-get wanted :effort)
                                     (romacs-agent-ui--default-effort session wanted))))
   romacs-agent-log-mode romacs-agent-compose-mode)
  (romacs-agent-log-in-chat
    (let ((session (romacs-agent-menu--session)))
      (unless (romacs-agent-select session (plist-get (romacs-agent--wanted session) :model) effort)
        (message "Effort is already %s" (or effort "the harness's"))))))

(defun romacs-agent-chat-model (model effort)
  "Run this session on MODEL, of its own instance, from its next turn on,
at EFFORT: the effort it runs at when MODEL takes it, else MODEL's default;
with a prefix argument, asked for."
  (interactive
   (let* ((session (romacs-agent-menu--session))
          (instance (romacs-agent--instance (romacs-agent-provider session) (romacs-agent-harness session)))
          (_ (romacs-agent--want-models (list instance)))
          (fields (cdr (romacs-agent--pick-row
                        (lambda ()
                          (romacs-agent--instance-rows
                           instance (romacs-agent--harness-display (romacs-agent-harness session)) nil)))))
          (model (plist-get fields :model))
          (efforts (plist-get fields :efforts))
          (now (plist-get (romacs-agent--wanted session) :effort))
          (fitting (if (member now efforts) now (plist-get fields :default-effort))))
     (list model (if (and current-prefix-arg efforts) (romacs-agent--read-effort efforts fitting) fitting)))
   romacs-agent-log-mode)
  (unless (romacs-agent-select (romacs-agent-menu--session) model effort)
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

(defun romacs-agent-menu--handoff-rows ()
  "The named models available for handoff, grouped by account."
  (seq-filter (lambda (row) (plist-get (cdr row) :model))
              (romacs-agent--account-rows)))

(defun romacs-agent-menu--handoff-default-effort (session fields same)
  "The default effort for FIELDS, retaining SESSION when SAME."
  (let ((provider (plist-get fields :provider)) (model (plist-get fields :model)))
    (or (if same
            (let ((running (romacs-agent--selection session)))
              (when (and (equal provider (plist-get running :provider))
                         (equal model (or (romacs-agent-model session) (plist-get running :model))))
                (romacs-agent-ui--default-effort session running)))
          (plist-get (romacs-agent--instance-options (romacs-agent--configured provider) (list :model model))
                     :effort))
        (plist-get fields :default-effort))))

(defun romacs-agent-menu--handoff-arguments ()
  "Read the target model, reasoning effort and next task for handoff."
  (let* ((fields (cdr (romacs-agent--pick-row #'romacs-agent-menu--handoff-rows "Harness and model")))
         (provider (plist-get fields :provider))
         (model (or (plist-get fields :model) (user-error "Choose a harness and model")))
         (session (romacs-agent-menu--session))
         (same (romacs-agent-menu--moves-p session provider))
         (wanted (romacs-agent--wanted session))
         (efforts (plist-get fields :efforts))
         (current (and (equal provider (plist-get wanted :provider))
                       (equal model (plist-get wanted :model))
                       (plist-get wanted :effort)))
         (default (romacs-agent-menu--handoff-default-effort session fields same))
         (effort (when efforts
                   (romacs-agent--read-effort efforts (if (member current efforts) current default) default))))
    (list provider model
          (unless same (read-string "Go on with: " nil nil "Go on from where this thread left off."))
          effort)))

(cl-defun romacs-agent-chat-handoff (provider model next &optional (effort nil effort-chosen))
  "Continue this thread on PROVIDER and MODEL at EFFORT.
Within a harness, change this session after its running turn ends.
Across harnesses, continue this chat buffer under a new session on a brief
of the thread and task NEXT."
  (interactive (romacs-agent-menu--handoff-arguments) romacs-agent-log-mode)
  (let ((session (romacs-agent-menu--session)))
    (cond
     ((not (romacs-agent-menu--moves-p session provider))
      (romacs-agent-menu--hand-off session provider model next effort))
     ((and (symbolp provider) (null model))
      (romacs-agent-chat-next-account))
     (t
      (let* ((wanted (romacs-agent--wanted session))
             (target (if (symbolp provider) (plist-get wanted :provider) provider))
             (model (or model (plist-get wanted :model)))
             (previous (plist-get wanted :effort))
             (instance (romacs-agent--instance target (romacs-agent-harness session))))
        (romacs-agent-select session model
                             (if effort-chosen effort
                               (and previous (romacs-agent--effort-fits-p instance model previous) previous))
                             target))))))

(defun romacs-agent-chat-next-account ()
  "Go on under the next account of this session's harness with room, its
conversation as is (`romacs-agent-move-on')."
  (interactive nil romacs-agent-log-mode)
  (let* ((session (romacs-agent-menu--session)) (to (romacs-agent-move-on session)))
    (message "%s goes on under %s, its conversation as is%s" (romacs-agent-title session) to
             (if (memq (romacs-agent-state session) '(starting running waiting-approval)) ", once this turn ends" ""))))

(defun romacs-agent-menu--moves-p (session provider)
  "Whether SESSION goes on under PROVIDER as it is: its harness's pool, or
an instance of its harness."
  (eq (if (symbolp provider) provider (plist-get (romacs-agent--configured provider) :harness))
      (romacs-agent-harness session)))

(defun romacs-agent-menu--handoff-position (session)
  "SESSION's journal and turn position while its handoff brief is read."
  (list (romacs-agent--journal-end session) (romacs-agent-seq session)
        (romacs-agent-state session) (romacs-agent-input-seq session)
        (romacs-agent-completed-seq session)))

(defun romacs-agent-menu--hand-off (session provider model next effort)
  "Start a session under PROVIDER on MODEL at EFFORT, briefed on SESSION.
Tell it to do NEXT."
  (when romacs-agent-log-queue--editing
    (user-error "Finish editing the queued message before switching harnesses"))
  (let ((source (current-buffer))
        (position (romacs-agent-menu--handoff-position session)))
    (message "Gathering %s's thread..." (romacs-agent-title session))
    (romacs-agent-events-async
     session
     (lambda (events)
       (when (buffer-live-p source)
         (with-current-buffer source
           (when (and (eq romacs-agent-log--session session) (not romacs-agent-log-queue--editing))
             (let ((thread (romacs-agent-handoff-thread session events)))
               (if (equal position (romacs-agent-menu--handoff-position session))
                   (romacs-agent-log-continue
                    (romacs-agent-handoff session thread provider model next (and effort (list :effort effort))))
                 (message "Handoff cancelled because the conversation changed. Switch harnesses again to include it."))))))))))

(defun romacs-agent-chat-plan ()
  "Put this session in its harness's plan mode, or take it out: what plan
mode lets it do is the harness's; a plan it presents asks you whether to
proceed."
  (interactive nil romacs-agent-log-mode)
  (let ((session (romacs-agent-menu--session)))
    (romacs-agent-set-plan session (not (plist-get (romacs-agent-options session) :plan)))))

(defun romacs-agent-menu--plan-label ()
  "The plan mode entry: whether the chat's session plans now."
  (if (and romacs-agent-log--session (plist-get (romacs-agent-options romacs-agent-log--session) :plan))
      (concat "Plan mode: " (propertize "on" 'face 'bold))
    (concat "Plan mode: " (propertize "off" 'face 'bold))))

(defun romacs-agent-chat-new ()
  "Start a new session, in this one's directory."
  (interactive nil romacs-agent-log-mode romacs-agent-compose-mode)
  (romacs-agent-log-in-chat
    (apply #'funcall-interactively #'romacs-agent-start
           (romacs-agent--start-arguments (file-name-as-directory (romacs-agent-cwd (romacs-agent-menu--session)))))))

(defun romacs-agent-menu--effort-label ()
  "The effort entry: the effort running, and the one to come if it differs."
  (let* ((session romacs-agent-log--session)
         (selection (and session (romacs-agent--selection session)))
         (wanted (and session (romacs-agent--wanted session)))
         (now (romacs-agent-menu--effort-text session selection))
         (next (if (equal selection wanted) now (romacs-agent-menu--effort-text session wanted))))
    (concat "Effort: " now (unless (equal now next) (concat " → " next)))))

(defun romacs-agent-menu--effort-text (session selection)
  "SELECTION's effort, including its level when it follows the default."
  (propertize (or (plist-get selection :effort)
                  (when-let* ((level (and session (romacs-agent-ui--default-effort session selection))))
                    (format "%s (default)" level))
                  "default")
              'face 'bold))

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
  (let* ((session (romacs-agent-menu--session))
         (request (or (cond (last (romacs-agent-menu--last-prompt session))
                            ;; From the composer, only the last turn, asked for.
                            ((>= (point) romacs-agent-log--end)
                             (user-error (substitute-command-keys
                                          "Put point on a turn, or \\[universal-argument] for the last")))
                            (t (romacs-agent-log-turns-here)))
                      (user-error "No turn here")))
         (files (seq-find (lambda (c) (and (equal (car c) request) (eq (nth 1 c) 'start)))
                          (romacs-agent-diff--checkpoints session))))
    (unless (let ((turn (truncate-string-to-width (or (romacs-agent-log-turns-text request) "the last turn") 50 nil nil t)))
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
  (let ((session (romacs-agent-menu--session)))
    (romacs-agent-diff-turn session (unless all (or (and (< (point) romacs-agent-log--end) (romacs-agent-log-turns-here))
                                                    (romacs-agent-diff-current session)
                                                    (user-error "No turn yet"))))))

(defun romacs-agent-chat-cancel-resume ()
  "This session no longer resumes by itself when its limit resets."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-tools--cancel-resume (romacs-agent-menu--session) "cancelled by hand"))

(defun romacs-agent-menu--link (file)
  "Put a link to FILE at the end of the composer, the current buffer."
  (goto-char (point-max))
  (unless (or (bobp) (memq (char-before) '(?\s ?\n))) (insert " "))
  (insert (romacs-agent-log-images-link file)))

(defun romacs-agent-chat-attach (&optional ask)
  "Link a file in the composer, [image](PATH) for an image: the agent
reads it where it is, and the chat shows it.  Interactively the
clipboard's image: the file it was copied from (a screenshot), else the
image saved once in `romacs-attach-save-directory'.  With a prefix
argument ASK, when the clipboard holds no image, or in a chat whose
session takes no commands from here, read a file name with image previews."
  (interactive "P" romacs-agent-log-mode romacs-agent-compose-mode)
  (romacs-agent-log-in-composer
    (if-let* ((type (and (not ask) (romacs-agent-log--commands-p) (romacs-attach-clipboard-image-type))))
        ;; At the composer's end, unless the draft is sent meanwhile.
        (when (romacs-agent-log-attach-paste type (point-max) t)
          (message "Attaching the clipboard's image…"))
      (let* ((romacs-complete-file-preview t)
             (cwd (and (romacs-agent-log--commands-p)
                       (romacs-agent-cwd (buffer-local-value 'romacs-agent-log--session romacs-agent-log--chat))))
             (dir (file-name-as-directory
                   (if (and cwd (not (file-remote-p cwd)) (file-directory-p cwd)) cwd default-directory))))
        (romacs-agent-menu--link (expand-file-name (read-file-name "Attach file: " dir nil t) dir))))))

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
  "A" ("Attach clipboard image (C-u: file)" romacs-agent-chat-attach)
  "u" ("Roll back this turn (C-u: last)" romacs-agent-chat-rollback
       :inapt-if (lambda () (not (romacs-agent--harness-free-p romacs-agent-log--session))))
  "D" ("Diff this turn (C-u: session)" romacs-agent-chat-diff)
  :group ("Queued" :if (lambda () (and romacs-agent-log--session
                                       (romacs-agent-log-source-queue romacs-agent-log--source romacs-agent-log--session))))
  "Q" ("Edit the queued message here" romacs-agent-log-queue-edit :inapt-if #'romacs-agent-log-queue-cannot-edit-p)
  "X" ("Drop the queued message here" romacs-agent-log-queue-drop :inapt-if #'romacs-agent-log-queue-cannot-drop-p)
  "<" ("Send it sooner" romacs-agent-log-queue-sooner :inapt-if (lambda () (romacs-agent-log-queue-not-here-p -1)))
  ">" ("Send it later" romacs-agent-log-queue-later :inapt-if (lambda () (romacs-agent-log-queue-not-here-p 1)))
  :group ("Runs on" :if #'romacs-agent-log--commands-p)
  "m" (#'romacs-agent-menu--model-label romacs-agent-chat-model)
  "E" (#'romacs-agent-menu--effort-label romacs-agent-chat-effort)
  "P" (#'romacs-agent-menu--plan-label romacs-agent-chat-plan)
  "H" ("Switch harness or model" romacs-agent-chat-handoff)
  "N" ("Next account with room" romacs-agent-chat-next-account)
  "z" ("Cancel the resume at its limit's reset" romacs-agent-chat-cancel-resume
       :if (lambda () (and romacs-agent-log--session (romacs-agent-resume-time romacs-agent-log--session))))
  ;; No :if: a group whose rows are all empty is not drawn, and a digit
  ;; whose worker settled since says so rather than falling through.
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
                    (and (> n 9) (format "All %d workers" n))))
       romacs-agent-menu-workers)
  :row
  :group ("Session" :if #'romacs-agent-log--commands-p)
  "b" ("Agent board" romacs-agents)
  "n" ("New session" romacs-agent-chat-new)
  "R" ("Rename" romacs-agent-chat-rename)
  "f" ("Fork" romacs-agent-chat-fork)
  "F" ("Fresh conversation, briefed" romacs-agent-chat-renew)
  "r" ("Resume" romacs-agent-chat-resume :inapt-if #'romacs-agent-log--running-p)
  "x" ("Stop" romacs-agent-chat-stop)
  :group "View"
  "e" ("Earlier turns" romacs-agent-log-earlier)
  "w" ("Copy the entry at point" romacs-agent-log-copy))

(put 'romacs-agent-menu-map-popup 'command-modes '(romacs-agent-log-mode))

(defun romacs-agent-log-menu ()
  "Open the chat's actions menu: from its composer, in the chat's window,
where what it offers is read and its actions run, the composer selected
again once the menu and what it ran are done
\(`romacs-agent-menu--refocus-after')."
  (interactive nil romacs-agent-log-mode romacs-agent-compose-mode)
  (if (not (derived-mode-p 'romacs-agent-compose-mode))
      (romacs-agent-menu-map-popup)
    (let ((from (selected-window)))
      (romacs-agent-compose-transcript)
      (romacs-agent-menu-map-popup)
      (romacs-agent-menu--refocus-after from (selected-window) (window-buffer)))))

(defun romacs-agent-menu--refocus-after (from window chat)
  "Once the menu open in WINDOW, CHAT's, closes, and the command it ran
ends with no minibuffer and no popup open (the menu again, another),
select FROM, its composer's window, again (`romacs-agent-log--refocus'):
once."
  (letrec ((ended (lambda ()
                    (unless (or (active-minibuffer-window) (keymap-popup-active-p))
                      (remove-hook 'post-command-hook ended)
                      (romacs-agent-log--refocus from window chat))))
           (closed (lambda ()
                     (remove-hook 'keymap-popup-exit-hook closed)
                     ;; It closes as its command starts: that command's end.
                     (add-hook 'post-command-hook ended))))
    (add-hook 'keymap-popup-exit-hook closed)))

(provide 'romacs-agent-menu)
;;; romacs-agent-menu.el ends here
