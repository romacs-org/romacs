;;; romacs-agent-switch.el --- Switch to an open agent chat  -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `romacs-agent-switch' reads one of the open chat buffers, those in
;; `romacs-agent-log-mode', through the completion UI (category
;; `romacs-agent-chat'), the most recently used first and the current
;; buffer last, each annotated with its session's state, harness and model,
;; and project.  The board (`romacs-agents') is the place for every thread;
;; this is the switch between the chats already open.

;;; Code:
(require 'cl-lib)
(require 'subr-x)
(require 'romacs-agent)
(require 'romacs-agent-summary)
(require 'romacs-agent-ui)
(require 'romacs-agent-log)

(declare-function romacs-agents "romacs-agent-board" ())

(defconst romacs-agent-switch--column 30
  "The widest a candidate's name pushes the annotations' column: a longer
name's annotation follows it.")

(defun romacs-agent-switch--chat-p (buffer)
  "Whether BUFFER is a live agent chat."
  (and (buffer-live-p buffer)
       (provided-mode-derived-p (buffer-local-value 'major-mode buffer) 'romacs-agent-log-mode)))

(defun romacs-agent-switch--named (name named)
  "The buffer NAMED, an alist of names and buffers, has under NAME: its
own, else the one whose name differs from NAME in case alone."
  (or (cdr (assoc name named))
      (let ((folded (seq-filter (lambda (entry) (string-equal-ignore-case name (car entry))) named)))
        (and (null (cdr folded)) (cdar folded)))))

(defun romacs-agent-switch--chats (buffers current)
  "The chat buffers among BUFFERS, in their order, CURRENT last when it is one."
  (let ((chats (seq-filter #'romacs-agent-switch--chat-p buffers)))
    (if (memq current chats) (append (delq current (copy-sequence chats)) (list current)) chats)))

(defun romacs-agent-switch--annotation (summary &optional pad)
  "What a chat's candidate says of its session's SUMMARY: its state, as
its chat's header says it, harness and model, and project, each in its
face, after PAD more columns of space."
  (if (null summary) ""
    (pcase-let* ((`(,_ ,glyph ,word ,face) (romacs-agent-ui--state-of (romacs-agent-summary-state-key summary)
                                                                       (romacs-agent-summary-watch-unit summary)))
                 (harness (romacs-agent-summary-harness summary))
                 (instance (romacs-agent-ui--instance
                            (and harness (symbol-name harness)) harness
                            (or (romacs-agent-summary-model summary) (romacs-agent-summary-selected-model summary))
                            nil))
                 (project (file-name-nondirectory (directory-file-name (or (romacs-agent-summary-cwd summary) "")))))
      (concat (make-string (+ 2 (or pad 0)) ?\s)
              (string-join (delq nil (list (if glyph (romacs-agent-ui--glyph glyph word face) (propertize word 'face face))
                                           (and (not (string-empty-p instance)) (propertize instance 'face 'romacs-agent-model))
                                           (and (not (string-empty-p project)) (propertize project 'face 'romacs-agent-dim))))
                           " ")))))

(defun romacs-agent-switch--properties (chats)
  "The completion metadata of CHATS' names (`completion-extra-properties'):
category `romacs-agent-chat', each annotated (`romacs-agent-switch--annotation')
from its session's summary, read once, the annotations in one column
\(`romacs-agent-switch--column' at most)."
  (let* ((summaries (make-hash-table :test #'equal))
         (width (min romacs-agent-switch--column
                     (apply #'max 0 (mapcar (lambda (chat) (string-width (buffer-name chat))) chats)))))
    (dolist (buffer chats)
      (puthash (buffer-name buffer) (with-current-buffer buffer (romacs-agent-log--summary)) summaries))
    (list :category 'romacs-agent-chat
          :annotation-function
          (lambda (name)
            (romacs-agent-switch--annotation (gethash name summaries) (max 0 (- width (string-width name))))))))

;;;###autoload
(defun romacs-agent-switch ()
  "Switch to an open agent chat in the selected window, read with
completion: the most recently used first, the current chat last.
With none open, offer the board (`romacs-agents')."
  (interactive)
  (if-let* ((chats (romacs-agent-switch--chats (buffer-list) (current-buffer))))
      (let* ((named (mapcar (lambda (buffer) (cons (buffer-name buffer) buffer)) chats))
             (name (romacs-agent--read-ordered (format-prompt "Agent chat" (caar named)) (mapcar #'car named)
                                               (romacs-agent-switch--properties chats) nil t nil nil (caar named)))
             (buffer (romacs-agent-switch--named name named)))
        (unless (romacs-agent-switch--chat-p buffer)
          (user-error "No open agent chat is named %s" name))
        (let ((switch-to-buffer-obey-display-actions nil))
          (switch-to-buffer buffer nil t)))
    (when (y-or-n-p "No agent chats are open.  Show the board? ")
      (romacs-agents))))

(provide 'romacs-agent-switch)
;;; romacs-agent-switch.el ends here
