;;; romacs-agent-log-queue.el --- What waits to be sent, in a chat  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The messages a chat's user queued (`romacs-agent-log-queue', C-c C-q)
;; wait in the session's queue, kept in the agents' store (one store:
;; the queue and its `queued' rows).  The chat shows them one row each
;; between its working thread and its composer, and the user changes
;; them there:
;;
;; - n and p move between them;
;; - e or RET puts one in the composer, held where it waits (it and what
;;   waits behind it wait), and RET there puts it back in its place, said
;;   anew; RET on an empty composer leaves it as it was;
;; - d drops one; M-p and M-n move it one sooner or later.
;;
;; The same commands are in the chat's menu.  Only the user's own
;; messages change: another session's message, or Romacs's own prompt,
;; shows but stays.  A followed chat's queue is changed where it runs.

;;; Code:

(require 'romacs-agent-log)

(defface romacs-agent-log-queued '((t :inherit romacs-agent-log-note))
  "A message waiting for the turn under way to end.")
(defface romacs-agent-log-queued-mark '((t :inherit romacs-working-waiting))
  "The mark of a queued message: the theme's colour for what waits.")

;;; The source's queue

(cl-defgeneric romacs-agent-log-source-queue (_source _session)
  "The messages waiting for SESSION, in order, as SOURCE knows them; none
by default: a followed chat's queue is where it runs."
  nil)

(cl-defgeneric romacs-agent-log-source-queue-change (_source _session _op _item &rest _args)
  "Change ITEM, waiting for SESSION, by OP (`hold', `edit', `drop',
`move') with ARGS, through SOURCE; non-nil when it was still waiting."
  (user-error "A followed chat's queue is changed where it runs"))

(cl-defmethod romacs-agent-log-source-queue ((_source romacs-agent-log-local-source) session)
  (romacs-agent-queue (romacs-agent-log--loaded session)))

(cl-defmethod romacs-agent-log-source-queue-change ((_source romacs-agent-log-local-source) session op item &rest args)
  (apply (pcase op
           ('hold #'romacs-agent-queue-hold) ('edit #'romacs-agent-queue-edit)
           ('drop #'romacs-agent-queue-drop) ('move #'romacs-agent-queue-move))
         (romacs-agent-log--loaded session) item args))

(defun romacs-agent-log-queue--change (op item &rest args)
  "Change ITEM by OP with ARGS (`romacs-agent-log-source-queue-change'):
a `user-error' when it went meanwhile, sent or dropped."
  (or (apply #'romacs-agent-log-source-queue-change romacs-agent-log--source romacs-agent-log--session op item args)
      (user-error "That message is no longer waiting")))

;;; Its rows

(defvar-keymap romacs-agent-log-queue-map
  :doc "Keys on a queued message's row."
  "n" #'romacs-agent-log-queue-next
  "p" #'romacs-agent-log-queue-previous
  "e" #'romacs-agent-log-queue-edit
  "RET" #'romacs-agent-log-queue-edit
  "d" #'romacs-agent-log-queue-drop
  "M-p" #'romacs-agent-log-queue-sooner
  "M-n" #'romacs-agent-log-queue-later)

(defun romacs-agent-log-queue-rows (items width)
  "ITEMS, the messages waiting, as rows WIDTH columns wide, each ending in
a newline; the empty string when none wait.  A row carries its item
\(`romacs-agent-log-queued') and the keys that change it."
  (mapconcat
   (lambda (item)
     (let* ((own (romacs-agent-queue-own-p item))
            (mark (cond ((romacs-agent-item-held item) "✎ ") (own "⏵ ") (t "← ")))
            (line (car (split-string (string-trim (or (romacs-agent-item-text item) "")) "\n")))
            (room (max 8 (- width 2 (string-width mark)))))
       (propertize (concat "  " (propertize mark 'face 'romacs-agent-log-queued-mark)
                           (propertize (truncate-string-to-width line room nil nil (truncate-string-ellipsis))
                                       'face (if own 'default 'romacs-agent-log-queued))
                           "\n")
                   'romacs-agent-log-queued item 'keymap romacs-agent-log-queue-map)))
   items ""))

(defun romacs-agent-log-queue--shown ()
  "The rows of what waits for this chat's session, for the separator."
  (romacs-agent-log-queue-rows (and romacs-agent-log--session
                                    (romacs-agent-log-source-queue romacs-agent-log--source romacs-agent-log--session))
                               (romacs-agent-log--rule-width)))

(setq romacs-agent-log-separator-rows-function #'romacs-agent-log-queue--shown)

;;; Moving among them

(defun romacs-agent-log-queue--item-at (&optional pos)
  "The queued message on the row at POS (point by default), or nil."
  (get-text-property (or pos (point)) 'romacs-agent-log-queued))

(defun romacs-agent-log-queue--goto (item)
  "Point to ITEM's row; nil when it shows none."
  (when-let* ((at (text-property-any romacs-agent-log--end romacs-agent-log--input 'romacs-agent-log-queued item)))
    (goto-char at)))

(defun romacs-agent-log-queue--target ()
  "The queued message to act on: the one at point, else the first; a
`user-error' when none waits."
  (or (romacs-agent-log-queue--item-at)
      (car (romacs-agent-log-source-queue romacs-agent-log--source romacs-agent-log--session))
      (user-error "Nothing is queued")))

(defun romacs-agent-log-queue--own (item)
  "ITEM, when its user may change it; else a `user-error'."
  (unless (romacs-agent-queue-own-p item)
    (user-error "Not yours to change: another session's message, or Romacs's own"))
  item)

(defun romacs-agent-log-queue--step (direction)
  "Point to the queued row DIRECTION (1 or -1) from point's."
  (let* ((items (romacs-agent-log-source-queue romacs-agent-log--source romacs-agent-log--session))
         (at (cl-position (romacs-agent-log-queue--item-at) items))
         (to (if at (+ at direction) (if (> direction 0) 0 (1- (length items))))))
    (if (and items (<= 0 to) (< to (length items)))
        (romacs-agent-log-queue--goto (nth to items))
      (user-error (if items "No more queued messages that way" "Nothing is queued")))))

(defun romacs-agent-log-queue-next ()
  "Move to the next queued message."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log-queue--step 1))

(defun romacs-agent-log-queue-previous ()
  "Move to the previous queued message."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log-queue--step -1))

;;; Changing them

(defun romacs-agent-log-queue-edit ()
  "Edit the queued message at point (else the first) in the composer: held
where it waits meanwhile; RET there puts it back in its place."
  (interactive nil romacs-agent-log-mode)
  (let ((item (romacs-agent-log-queue--own (romacs-agent-log-queue--target))))
    (when romacs-agent-log-queue--editing
      (user-error "The composer holds a queued message already: RET puts it back"))
    (unless (string-blank-p (buffer-substring-no-properties romacs-agent-log--input (point-max)))
      (user-error "The composer holds a draft: send or clear it first"))
    (romacs-agent-log-queue--change 'hold item t)
    (setq romacs-agent-log-queue--editing item)
    (goto-char (point-max))
    (insert (romacs-agent-item-text item))
    (message "Editing a queued message: RET puts it back in its place, said anew")))

(defun romacs-agent-log-queue-put-back ()
  "Put the message the composer holds back in its place in the queue, said
as the composer says it; an empty composer leaves it as it was."
  (interactive nil romacs-agent-log-mode)
  (let ((item (or romacs-agent-log-queue--editing (user-error "The composer holds no queued message")))
        (text (string-trim (buffer-substring-no-properties romacs-agent-log--input (point-max)))))
    ;; Held to edit until it is back: a failure leaves the composer as it
    ;; is, still putting it back.
    (if (string-empty-p text)
        (romacs-agent-log-queue--change 'hold item nil)
      (romacs-agent-log-queue--change 'edit item text)
      (romacs-agent-log--writing (delete-region romacs-agent-log--input (point-max))))
    (setq romacs-agent-log-queue--editing nil)
    (romacs-agent-log--separator t)
    (romacs-agent-log-queue--goto item)))

(defun romacs-agent-log-queue-drop ()
  "Drop the queued message at point (else the first), unsent."
  (interactive nil romacs-agent-log-mode)
  (let* ((item (romacs-agent-log-queue--own (romacs-agent-log-queue--target)))
         (items (romacs-agent-log-source-queue romacs-agent-log--source romacs-agent-log--session))
         (next (or (cadr (memq item items)) (car (last (remq item items))))))
    (when (eq item romacs-agent-log-queue--editing)
      (user-error "It is in the composer: RET puts it back first"))
    (romacs-agent-log-queue--change 'drop item)
    (romacs-agent-log--separator t)
    (when next (romacs-agent-log-queue--goto next))))

(defun romacs-agent-log-queue--move (delta)
  "Move the queued message at point (else the first) DELTA places."
  (let* ((item (romacs-agent-log-queue--own (romacs-agent-log-queue--target)))
         (items (romacs-agent-log-source-queue romacs-agent-log--source romacs-agent-log--session))
         (to (+ (or (cl-position item items) 0) delta)))
    (cond ((< to 0) (user-error "It goes first already"))
          ((>= to (length items)) (user-error "It goes last already")))
    (romacs-agent-log-queue--change 'move item delta)
    (romacs-agent-log--separator t)
    (romacs-agent-log-queue--goto item)))

(defun romacs-agent-log-queue-sooner ()
  "Move the queued message at point (else the first) one place sooner."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log-queue--move -1))

(defun romacs-agent-log-queue-later ()
  "Move the queued message at point (else the first) one place later."
  (interactive nil romacs-agent-log-mode)
  (romacs-agent-log-queue--move 1))

(defun romacs-agent-log-queue--release ()
  "A chat that goes lets go of the message it held to edit, unchanged."
  (when (and (derived-mode-p 'romacs-agent-log-mode) romacs-agent-log-queue--editing)
    (ignore-error user-error
      (romacs-agent-log-source-queue-change romacs-agent-log--source romacs-agent-log--session
                                            'hold romacs-agent-log-queue--editing nil))
    (setq romacs-agent-log-queue--editing nil)))

(add-hook 'kill-buffer-hook #'romacs-agent-log-queue--release)
(add-hook 'change-major-mode-hook #'romacs-agent-log-queue--release)

(provide 'romacs-agent-log-queue)
;;; romacs-agent-log-queue.el ends here
