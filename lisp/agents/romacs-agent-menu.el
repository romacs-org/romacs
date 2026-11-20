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

(declare-function romacs-agent-ui--model-label "romacs-agent-ui" ())
(declare-function romacs-agent-ui--effort-label "romacs-agent-ui" ())
(declare-function romacs-agent-ui--plan-label "romacs-agent-ui" ())
(declare-function romacs-agent-chat-model "romacs-agent-ui" (model effort))
(declare-function romacs-agent-chat-effort "romacs-agent-ui" (effort))
(declare-function romacs-agent-chat-plan "romacs-agent-ui" ())
(declare-function romacs-agent-chat-handoff "romacs-agent-ui" (provider model next))
(declare-function romacs-agent-chat-next-account "romacs-agent-ui" ())
(declare-function romacs-agent-chat-new "romacs-agent-ui" ())
(declare-function romacs-agent-chat-rename "romacs-agent-ui" (title))
(declare-function romacs-agent-chat-fork "romacs-agent-ui" ())
(declare-function romacs-agent-chat-renew "romacs-agent-ui" ())
(declare-function romacs-agent-chat-resume "romacs-agent-ui" ())
(declare-function romacs-agent-chat-stop "romacs-agent-ui" ())
(declare-function romacs-agents "romacs-agent-board" ())

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
  :group "Runs on"
  "m" (#'romacs-agent-ui--model-label romacs-agent-chat-model)
  "E" (#'romacs-agent-ui--effort-label romacs-agent-chat-effort)
  "P" (#'romacs-agent-ui--plan-label romacs-agent-chat-plan)
  "H" ("Go on under another instance" romacs-agent-chat-handoff)
  "N" ("Next account with room" romacs-agent-chat-next-account)
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
  "w" ("Copy the entry at point" romacs-agent-log-copy))

(put 'romacs-agent-menu-map-popup 'command-modes '(romacs-agent-log-mode))

(provide 'romacs-agent-menu)
;;; romacs-agent-menu.el ends here
