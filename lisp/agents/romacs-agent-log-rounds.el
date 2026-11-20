;;; romacs-agent-log-rounds.el --- An agent chat's rounds, told at a glance  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A chat (romacs-agent-log.el) is a run of rounds: a message, and all
;; the agent did in answer.  Who each block is reads by shape and colour,
;; before any word: every line of a block carries its kind's gutter, a
;; `line-prefix' and `wrap-prefix' of one bar and a space, which draw
;; beside the text and are no part of it.
;;
;; - the user's message opens a round: an accent bar, its lines on a
;;   tinted surface;
;; - a message from elsewhere (another session's, a delegated task's
;;   result, a background task's notice) opens one too, but is not the
;;   user: a dim broken bar, a head saying whom it is from, its text dim;
;; - the agent's work keeps its rows; its reply is the text as it is.
;; When a round ends, the agent's words before its last are dimmed: its
;; answer stands out by what recedes, as in T3 Code's timeline (design:
;; the chat lane's rounds-design.md).

;;; Code:

(require 'romacs-agent-log)

(defface romacs-agent-log-you-bar '((t :inherit romacs-working-running))
  "The gutter of the user's messages: the theme's colour for work, never
a syntax face's.")
(defface romacs-agent-log-you '((t :extend t))
  "The surface of the user's messages: the background moved a little
toward the text, derived on each theme change.")
(defface romacs-agent-log-other-bar '((t :inherit romacs-agent-log-note))
  "The gutter of what came from elsewhere.")
(defface romacs-agent-log-other '((t :inherit romacs-agent-log-note))
  "The text of what came from elsewhere: dim, never the user's or the
agent's own.")
(defface romacs-agent-log-other-head '((t :inherit romacs-agent-log-note :weight bold))
  "Whom a message from elsewhere is from.")

(defcustom romacs-agent-log-rounds-other-lines 3
  "How many lines of a message from elsewhere show; RET shows all."
  :type 'natnum
  :group 'romacs-agent)

(defun romacs-agent-log-rounds--derive (&rest _)
  "Make the user's surface the background mixed toward the text, as its
default spec: told from the transcript on every theme, light or dark."
  (let* ((bg (romacs-color-hex (face-background 'default nil t)))
         (fg (romacs-color-hex (face-foreground 'default nil t)))
         (surface (and bg fg (romacs-color-mix bg fg 0.08))))
    (face-spec-set 'romacs-agent-log-you
                   (if surface `((t :background ,surface :extend t)) '((t :extend t)))
                   'face-defface-spec)))

(add-hook 'enable-theme-functions #'romacs-agent-log-rounds--derive)
(add-hook 'disable-theme-functions #'romacs-agent-log-rounds--derive)
(romacs-agent-log-rounds--derive)

(defun romacs-agent-log-rounds--gutter (text bar face)
  "TEXT with every line behind BAR in FACE, as display: line and wrap
prefixes, no part of the text."
  (let ((prefix (propertize (concat bar " ") 'face face)))
    (propertize text 'line-prefix prefix 'wrap-prefix prefix)))

(defun romacs-agent-log-rounds-you (text)
  "TEXT, the user's message as a chat draws it, as its round's opener:
its gutter and its surface."
  (let ((text (copy-sequence text)))
    (add-face-text-property 0 (length text) 'romacs-agent-log-you t text)
    (romacs-agent-log-rounds--gutter text "▌" 'romacs-agent-log-you-bar)))

(defun romacs-agent-log-rounds--head (incoming)
  "INCOMING's head: a mark, whom it is from, and its state."
  (let ((state (plist-get incoming :state)))
    (concat (propertize "✉ " 'face 'romacs-agent-log-other-bar)
            (propertize (or (plist-get incoming :from) "elsewhere") 'face 'romacs-agent-log-other-head)
            (if (and state (not (string-empty-p state)))
                (propertize (concat " · " state) 'face 'romacs-agent-log-other)
              ""))))

(defun romacs-agent-log-rounds-incoming (event incoming)
  "Draw INCOMING, from EVENT, a message from elsewhere: a round of its own,
not the user's; its first lines dim, RET shows all."
  (let* ((body (string-trim (or (plist-get incoming :text) "")))
         (lines (split-string body "\n"))
         (shown (seq-take lines romacs-agent-log-rounds-other-lines))
         (more (- (length lines) (length shown)))
         (text (concat "\n" (romacs-agent-log-rounds--head incoming) "\n"
                       (propertize (string-join shown "\n") 'face 'romacs-agent-log-other)
                       (if (> more 0)
                           (propertize (format "\n… %d more line%s · %s shows all" more (if (= more 1) "" "s")
                                               (romacs-agent-log--key 'romacs-agent-log-open))
                                       'face 'romacs-agent-log-note)
                         ""))))
    (romacs-agent-log--line event 'incoming
                            (propertize (romacs-agent-log-rounds--gutter text "┆" 'romacs-agent-log-other-bar)
                                        'romacs-agent-log-full (cons nil body)))))

(defun romacs-agent-log-rounds--start ()
  "Where the round now ending began: its opener's entry, else the start."
  (let ((at romacs-agent-log--end) (found nil))
    (while (and (not found) (setq at (previous-single-property-change at 'romacs-agent-log-entry nil (point-min)))
                (> at (point-min)))
      (when (memq (get-text-property at 'romacs-agent-log-entry) '(prompt incoming))
        (setq found at)))
    (or found (point-min))))

(defun romacs-agent-log-rounds-end ()
  "The round ending: the agent's words before its last recede, so its
answer stands out.  Faces only: no text moves."
  (let ((texts nil) (at (romacs-agent-log-rounds--start)))
    (while (setq at (text-property-any at romacs-agent-log--end 'romacs-agent-log-entry 'text))
      (push at texts)
      (setq at (1+ at)))
    (romacs-agent-log--writing
      (dolist (start (cdr texts))
        ;; An entry is marked at its first character: it runs to the next's.
        (add-face-text-property start (or (text-property-not-all (1+ start) romacs-agent-log--end 'romacs-agent-log-entry nil)
                                          romacs-agent-log--end)
                                'romacs-agent-log-other)))))

(provide 'romacs-agent-log-rounds)
;;; romacs-agent-log-rounds.el ends here
