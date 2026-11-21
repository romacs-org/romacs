;;; romacs-agent-log-rounds.el --- An agent chat's rounds, told at a glance  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; A chat (romacs-agent-log.el) is a run of rounds: a message, and all
;; the agent did in answer.  Who each block is reads by shape and colour,
;; before any word:
;;
;; - the user's message opens a round: its lines on a tinted surface,
;;   behind a thick bar in the colour that means the user everywhere
;;   (`romacs-working-waiting', what waits on them on the board);
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
(require 'romacs-markdown)

(defface romacs-agent-log-you '((t :extend t))
  "The surface of the user's messages: the background moved a little
toward the text, derived on each theme change.")
(defface romacs-agent-log-you-top '((t :overline t :extend t))
  "The hairline over a round's first line, the user's message: where a
round begins, found at a glance as the chat scrolls.  Its colour is the
theme's dim text, derived on each theme change.")
(defface romacs-agent-log-you-bar '((t :inherit romacs-working-waiting))
  "The bar the user's messages stand behind: the colour that means the
user, what waits on them, on every view of agents.")
;; The user's message as named faces, each its parts' (romacs#377).
(defface romacs-agent-log-you-bar-first
  '((t :inherit (romacs-agent-log-you-top romacs-agent-log-you-bar romacs-agent-log-you)))
  "The bar before the first line of the user's message.")
(defface romacs-agent-log-you-bar-line '((t :inherit (romacs-agent-log-you-bar romacs-agent-log-you)))
  "The bar before another line of the user's message.")
(defface romacs-agent-log-you-text-first
  '((t :inherit (romacs-agent-log-you-top romacs-agent-log-prompt romacs-agent-log-you)))
  "The first line of the user's message, under its round's hairline.")
(defface romacs-agent-log-you-text '((t :inherit (romacs-agent-log-prompt romacs-agent-log-you)))
  "Another line of the user's message.")
(defface romacs-agent-log-you-note-first
  '((t :inherit (romacs-agent-log-you-top romacs-agent-log-note romacs-agent-log-you)))
  "A note in the first line of the user's message (how many images).")
(defface romacs-agent-log-you-note '((t :inherit (romacs-agent-log-note romacs-agent-log-you)))
  "A note in another line of the user's message.")
(defface romacs-agent-log-other-mark '((t :inherit romacs-working-running))
  "The mark of what came from elsewhere, and the bar it is quoted behind:
the theme's colour for work.")
(defface romacs-agent-log-other '((t :inherit romacs-agent-log-note))
  "The text of what came from elsewhere: dim, never the user's or the
agent's own.")
(defface romacs-agent-log-other-head '((t :inherit romacs-working-running :weight bold))
  "Whom a message from elsewhere is from: the theme's colour for work.")

(defcustom romacs-agent-log-rounds-other-lines 2
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
                   'face-defface-spec)
    ;; A hairline, never a rule: the dim text's colour, toward the surface.
    (let ((line (romacs-color-hex (face-foreground 'shadow nil t))))
      (face-spec-set 'romacs-agent-log-you-top
                     (if line `((t :overline ,line :extend t)) '((t :overline t :extend t)))
                     'face-defface-spec))))

(add-hook 'enable-theme-functions #'romacs-agent-log-rounds--derive)
(add-hook 'disable-theme-functions #'romacs-agent-log-rounds--derive)
(romacs-agent-log-rounds--derive)

(defun romacs-agent-log-rounds--you-line (line first)
  "LINE of the user's message, its runs in the named faces of its surface:
the prompt's (its mark's too) and a note's, the FIRST line's under the
round's hairline.  Named faces, never lists: the display realizes each
once (romacs#377), and a line is two runs, its bar and its words."
  (let ((line (copy-sequence line)) (at 0) (end (length line)))
    (while (< at end)
      (let ((next (next-single-property-change at 'face line end)))
        (put-text-property at next 'face
                           (if (eq (get-text-property at 'face line) 'romacs-agent-log-note)
                               (if first 'romacs-agent-log-you-note-first 'romacs-agent-log-you-note)
                             (if first 'romacs-agent-log-you-text-first 'romacs-agent-log-you-text))
                           line)
        (setq at next)))
    (concat (propertize "▌ " 'face (if first 'romacs-agent-log-you-bar-first 'romacs-agent-log-you-bar-line)
                        'romacs-agent-decor t)
            line
            ;; Its line break on its surface, to the window's edge.
            (propertize "\n" 'face (if first 'romacs-agent-log-you-text-first 'romacs-agent-log-you-text)))))

(defun romacs-agent-log-rounds-you (text)
  "TEXT, the user's message as a chat draws it, as its round's opener:
each of its lines behind its bar on its surface, a block to the
window's edge, the first under the round's hairline; the blank lines
before and after it stay the transcript's.  The round before it ends."
  (romacs-agent-log-rounds-end)
  (let* ((text (if (string-suffix-p "\n" text) (substring text 0 -1) text))
         (first (or (string-match "[^\n]" text) 0))
         (n -1)
         (text (concat (substring text 0 first)
                       ;; Blank lines between its own too: its bar runs on.
                       (mapconcat (lambda (line) (romacs-agent-log-rounds--you-line line (zerop (setq n (1+ n)))))
                                  (split-string (substring text first) "\n") ""))))
    (put-text-property 0 1 'romacs-agent-log-round t text)
    (concat text "\n")))

(defun romacs-agent-log-rounds--words (text width)
  "TEXT in WIDTH columns, cut at a word's end before an ellipsis, never
inside one (unless its first word is longer)."
  (if (<= (string-width text) width) text
    (let* ((ellipsis (truncate-string-ellipsis))
           (cut (truncate-string-to-width text (- width (string-width ellipsis))))
           ;; Cut inside a word: back to the end of the one before.
           (word (and (< (length cut) (length text)) (not (memq (aref text (length cut)) '(?\s ?\t)))
                      (string-match "[ \t]+[^ \t]*\\'" cut))))
      ;; Ends as a word does: no lone operator or separator before the
      ;; ellipsis (one a word ends with, as C++'s, stays).
      (concat (string-trim-right (if (and word (> word 0)) (substring cut 0 word) cut)
                                 "\\(?:[ \t]+[^[:alnum:] \t]+\\)*[ \t:,;·]*")
              ellipsis))))

(defun romacs-agent-log-rounds--head (incoming)
  "INCOMING's head: a mark, whom it is from, and its state."
  (let ((state (plist-get incoming :state)))
    (concat (propertize "← " 'face 'romacs-agent-log-other-mark)
            ;; Whom it is from, a session's title as long as a name.
            (propertize (romacs-agent-log-rounds--words (or (plist-get incoming :from) "elsewhere") 60)
                        'face 'romacs-agent-log-other-head)
            (if (and state (not (string-empty-p state)))
                (propertize (concat " · " state) 'face 'romacs-agent-log-other)
              ""))))

(defun romacs-agent-log-rounds-incoming (event incoming)
  "Draw INCOMING, from EVENT, a message from elsewhere: a round of its own,
not the user's; its first lines dim, RET shows all."
  (let* ((bar (propertize "┃ " 'face 'romacs-agent-log-other-mark 'romacs-agent-decor t))
         ;; Under its call, if that is drawn; else at the top level.
         (parent (car (romacs-agent-log--where event)))
         ;; A row's room less its bar and a subagent's indentation.
         (width (- romacs-agent-log-output-width (length bar) (* 2 (cdr (romacs-agent-log--where event)))))
         (body (string-trim (or (plist-get incoming :text) "")))
         ;; Its lines that say something, as they read rendered.
         ;; Read as one document (a span or a fence across its lines),
         ;; its first part only: the preview shows a few lines.
         (lines (split-string (romacs-markdown-plain-string (substring body 0 (min (length body) 4096))) "\n" t "[ \t]*"))
         (shown (seq-take lines romacs-agent-log-rounds-other-lines))
         ;; How many lines it leaves out, when all of it was read.
         (more (and (<= (length body) 4096) (- (length lines) (length shown))))
         ;; One blank line before it: an arrival before ends in its own.
         ;; Its bar from its head down: the mark of what came in.
         (text (concat (if (and (not parent) (romacs-agent-log--blank-p)) "" "\n")
                       bar (romacs-agent-log-rounds--head incoming) "\n"
                       ;; Quoted, each line one row, so its bar runs unbroken.
                       ;; In its Markdown faces, dimmed over them: not the user's
                       ;; or the agent's own words.
                       (mapconcat (lambda (line)
                                    (let ((line (truncate-string-to-width line width nil nil (truncate-string-ellipsis))))
                                      ;; Before its faces: the dim colour wins, they
                                      ;; keep their weight and slant.
                                      (add-face-text-property 0 (length line) 'romacs-agent-log-other nil line)
                                      (concat bar line)))
                                  shown "\n")
                       ;; Whatever it leaves out, lines or the ends of lines,
                       ;; RET shows.
                       (if (or (not more) (> more 0) (seq-some (lambda (line) (> (string-width line) width)) shown))
                           (concat "\n" bar
                                   (propertize (concat (if (and more (> more 0)) (format "… %d more line%s · " more (if (= more 1) "" "s")) "… ")
                                                       (romacs-agent-log--key 'romacs-agent-log-open) " shows all")
                                               'face 'romacs-agent-log-note))
                         "")
                       ;; Apart from what the agent says to it; under a
                       ;; call, in its lines.
                       (if parent "\n" "\n\n"))))
    ;; A top-level arrival opens a round, the one before it ended, under
    ;; the hairline every round begins with; one under a subagent's call
    ;; is that call's.
    (unless parent
      (romacs-agent-log-rounds-end)
      (let* ((head (if (string-prefix-p "\n" text) 1 0))
             (end (min (length text) (1+ (or (string-search "\n" text head) (length text))))))
        (add-face-text-property head end 'romacs-agent-log-you-top t text)))
    (romacs-agent-log--line event 'arrival
                            (let ((text (propertize (romacs-agent-log--shared-faces text)
                                                    'romacs-agent-log-full (cons nil (or (plist-get incoming :full) body)))))
                              (unless parent
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
