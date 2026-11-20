;;; romacs-agent-log-rounds.el --- An agent chat's rounds, told at a glance  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A chat (romacs-agent-log.el) is a run of rounds: a message, and all
;; the agent did in answer.  Who each block is reads by shape and colour,
;; before any word:
;;
;; - the user's message opens a round: its lines on a tinted surface;
;; - a message from elsewhere (another session's, a delegated task's
;;   result, a background task's notice) opens one too, but is not the
;;   user: quoted behind a bar from its head, ← and whom it is from, to
;;   its first lines, dim, a blank line after;
;; - the agent's work keeps its rows; its reply is the text as it is.
;; When a round ends, the agent's words before its last are dimmed: its
;; answer stands out by what recedes, as in T3 Code's timeline (design:
;; the chat lane's rounds-design.md).

;;; Code:

(require 'romacs-agent-log)

(defface romacs-agent-log-you '((t :extend t))
  "The surface of the user's messages: the background moved a little
toward the text, derived on each theme change.")
(defface romacs-agent-log-other-mark '((t :inherit romacs-working-running))
  "The mark of what came from elsewhere, and the bar it is quoted behind:
the theme's colour for work.")
(defface romacs-agent-log-other '((t :inherit romacs-agent-log-note))
  "The text of what came from elsewhere: dim, never the user's or the
agent's own.")
(defface romacs-agent-log-other-head '((t :inherit romacs-working-running :weight bold))
  "Whom a message from elsewhere is from: the theme's colour for work.")

(defcustom romacs-agent-log-rounds-other-lines 3
  "How many lines of a message from elsewhere show; RET shows all."
  :type 'natnum
  :group 'romacs-agent)

(defun romacs-agent-log-rounds--derive (&rest _)
  "Make the user's surface the background mixed toward the text until it
stands apart (`romacs-color-band'), as its default spec: told from the
transcript on every theme, light, dark or black."
  (let* ((bg (romacs-color-hex (face-background 'default nil t)))
         (fg (romacs-color-hex (face-foreground 'default nil t)))
         (surface (and bg fg (romacs-color-band bg fg 1.3))))
    (face-spec-set 'romacs-agent-log-you
                   (if surface `((t :background ,surface :extend t)) '((t :extend t)))
                   'face-defface-spec)))

(add-hook 'enable-theme-functions #'romacs-agent-log-rounds--derive)
(add-hook 'disable-theme-functions #'romacs-agent-log-rounds--derive)
(romacs-agent-log-rounds--derive)

(defun romacs-agent-log-rounds-you (text)
  "TEXT, the user's message as a chat draws it, as its round's opener:
its surface, a block to the window's edge from its first line through
its newline; the blank lines before and after it stay the transcript's.
The round before it ends."
  (romacs-agent-log-rounds-end)
  (let ((text (if (string-suffix-p "\n" text) (copy-sequence text) (concat text "\n"))))
    (put-text-property 0 1 'romacs-agent-log-round t text)
    (add-face-text-property (or (string-match "[^\n]" text) 0) (length text) 'romacs-agent-log-you t text)
    (concat text "\n")))

(defun romacs-agent-log-rounds--head (incoming)
  "INCOMING's head: a mark, whom it is from, and its state."
  (let ((state (plist-get incoming :state)))
    (concat (propertize "← " 'face 'romacs-agent-log-other-mark)
            ;; Whom it is from, a session's title as long as a name.
            (propertize (truncate-string-to-width (or (plist-get incoming :from) "elsewhere") 40 nil nil (truncate-string-ellipsis))
                        'face 'romacs-agent-log-other-head)
            (if (and state (not (string-empty-p state)))
                (propertize (concat " · " state) 'face 'romacs-agent-log-other)
              ""))))

(defun romacs-agent-log-rounds-incoming (event incoming)
  "Draw INCOMING, from EVENT, a message from elsewhere: a round of its own,
not the user's; its first lines dim, RET shows all."
  (let* ((bar (propertize "┃ " 'face 'romacs-agent-log-other-mark 'romacs-agent-decor t))
         (parent (plist-get event :parent))
         ;; A row's room less its bar and a subagent's indentation.
         (width (- romacs-agent-log-output-width (length bar) (* 2 (cdr (romacs-agent-log--where event)))))
         (body (string-trim (or (plist-get incoming :text) "")))
         (lines (split-string body "\n"))
         (shown (seq-take lines romacs-agent-log-rounds-other-lines))
         (more (- (length lines) (length shown)))
         ;; One blank line before it: an arrival before ends in its own.
         ;; Its bar from its head down: the mark of what came in.
         (text (concat (if (and (not parent) (> romacs-agent-log--end (+ (point-min) 1))
                                (equal (buffer-substring-no-properties (- romacs-agent-log--end 2) romacs-agent-log--end) "\n\n"))
                           ""
                         "\n")
                       bar (romacs-agent-log-rounds--head incoming) "\n"
                       ;; Quoted, each line one row, so its bar runs unbroken.
                       (mapconcat (lambda (line)
                                    (concat bar (propertize (truncate-string-to-width line width nil nil (truncate-string-ellipsis))
                                                            'face 'romacs-agent-log-other)))
                                  shown "\n")
                       (if (> more 0)
                           (concat "\n" bar
                                   (propertize (format "… %d more line%s · %s shows all" more (if (= more 1) "" "s")
                                                       (romacs-agent-log--key 'romacs-agent-log-open))
                                               'face 'romacs-agent-log-note))
                         "")
                       ;; Apart from what the agent says to it; under a
                       ;; call, in its lines.
                       (if parent "\n" "\n\n"))))
    ;; A top-level arrival opens a round, the one before it ended; one
    ;; under a subagent's call is that call's.
    (unless (plist-get event :parent) (romacs-agent-log-rounds-end))
    (romacs-agent-log--line event 'arrival
                            (let ((text (propertize text 'romacs-agent-log-full (cons nil (or (plist-get incoming :full) body)))))
                              (unless (plist-get event :parent)
                                (put-text-property 0 1 'romacs-agent-log-round t text))
                              ;; A journaled input's turn: diff and rollback read it.
                              (if-let* ((request (plist-get event :request)))
                                  (propertize text 'romacs-agent-log-turn request)
                                text)))))

(defun romacs-agent-log-rounds--start ()
  "Where the round now ending began: its top-level opener, else the start."
  (let ((at romacs-agent-log--end) (found nil))
    (while (and (not found) (setq at (previous-single-property-change at 'romacs-agent-log-round nil (point-min)))
                (> at (point-min)))
      (when (get-text-property at 'romacs-agent-log-round) (setq found at)))
    (or found (and (get-text-property (point-min) 'romacs-agent-log-round) (point-min)) (point-min))))

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
        ;; Receded once: a round ends at its turn's end and again as the
        ;; next opens.
        (unless (memq 'romacs-agent-log-other (ensure-list (get-text-property start 'face)))
          (add-face-text-property start (or (text-property-not-all (1+ start) romacs-agent-log--end 'romacs-agent-log-entry nil)
                                            romacs-agent-log--end)
                                  'romacs-agent-log-other))))))

(provide 'romacs-agent-log-rounds)
;;; romacs-agent-log-rounds.el ends here
