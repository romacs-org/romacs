;;; romacs-markdown-mode.el --- Editing Markdown -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `romacs-markdown-mode' edits Markdown files on romacs-markdown.el's
;; mechanism.  Font Lock faces only the blocks that hold what redisplay
;; asks for: the buffer's block index (Rust) knows where its top-level
;; blocks start and follows its edits, so a key reparses the block it
;; changed, and the text a change reshaped further on (a fence opened, a
;; list joined) is faced again as it comes into view.
;;
;; - Headings are the outline: `outline-minor-mode' and imenu use them, and
;;   C-c C-n, C-c C-p, C-c C-u, C-c C-f and C-c C-b move between them.  A
;;   `#' line is a heading only where a top-level block starts, so a shell
;;   comment in a code block is none.
;; - M-RET continues the list item at point; C-c C-c aligns the table at
;;   point; C-c C-o follows the link at point.
;; - `romacs-markdown-view-mode' shows Markdown read-only.

;;; Code:

(require 'romacs-markdown)
(require 'outline)
(require 'seq)

(declare-function romacs-markdown--fontify-buffer "markdown.rs" (beg end faces))
(declare-function romacs-markdown--block-at "markdown.rs" (pos))
(declare-function romacs-markdown--links "markdown.rs" (beg end))
(declare-function romacs-markdown--table-align "markdown.rs" (source))

;;; Font Lock

(defun romacs-markdown--font-lock-region (beg end &optional _loudly)
  "Font Lock's region function: face the blocks that hold BEG..END."
  (with-silent-modifications
    (pcase-let ((`(,start ,stop . ,codes) (romacs-markdown--fontify-buffer beg end romacs-markdown-faces)))
      (romacs-markdown--colour-codes codes)
      `(jit-lock-bounds ,start . ,stop))))

;;; Outline

(defconst romacs-markdown--heading-regexp "^#\\{1,6\\}[ \t]"
  "An ATX heading's start, where a top-level block starts.")

(defun romacs-markdown--heading-start-p (pos)
  "Whether a top-level block, so a heading when its line opens with `#'s,
starts at POS."
  (= (car (romacs-markdown--block-at pos)) pos))

(defun romacs-markdown--outline-search (&optional bound move backward looking-at)
  "`outline-search-function': search for the next heading, as BOUND, MOVE,
BACKWARD and LOOKING-AT say."
  (if looking-at
      (and (looking-at romacs-markdown--heading-regexp) (romacs-markdown--heading-start-p (point)))
    (let ((start (point)) found)
      (while (and (not found)
                  (if backward
                      (re-search-backward romacs-markdown--heading-regexp bound t)
                    (re-search-forward romacs-markdown--heading-regexp bound t)))
        (when (save-match-data (romacs-markdown--heading-start-p (match-beginning 0)))
          (setq found t)))
      (cond (found t)
            (move (goto-char (or bound (if backward (point-min) (point-max)))) nil)
            (t (goto-char start) nil)))))

(defun romacs-markdown--outline-level ()
  "The heading at point's level: its `#'s."
  (save-excursion
    (beginning-of-line)
    (skip-chars-forward "#")
    (- (point) (line-beginning-position))))

(defun romacs-markdown--imenu ()
  "The buffer's headings, for imenu: each title with its position."
  (save-restriction
    (widen)
    (delq nil (mapcar (pcase-lambda (`(,kind ,beg ,end . ,_))
                        (when (eq kind 'heading)
                          (cons (string-trim (replace-regexp-in-string
                                              "\\`#+[ \t]*\\|[ \t]+#+[ \t]*$\\|\n[=-]+[ \t]*\n?\\'" ""
                                              (buffer-substring-no-properties beg end)))
                                beg)))
                      (romacs-markdown-blocks (point-min) (point-max))))))

;;; Lists

(defconst romacs-markdown--item-regexp
  "^\\([ \t]*\\)\\([-*+]\\|\\([0-9]+\\)\\([.)]\\)\\)\\([ \t]+\\)\\(\\[[ xX]\\][ \t]+\\)?"
  "A list item's indentation, marker (its number and delimiter when
ordered), spacing and task box.")

(defun romacs-markdown-insert-item ()
  "Start a new list item after the one at point, as it is marked: the next
number when ordered, an empty box when a task.  Outside a list, a newline."
  (interactive "*" romacs-markdown-mode)
  (let ((line (buffer-substring-no-properties (line-beginning-position) (line-end-position))))
    (if (not (string-match romacs-markdown--item-regexp line))
        (newline)
      (let ((indent (match-string 1 line))
            (marker (if (match-string 3 line)
                        (format "%d%s" (1+ (string-to-number (match-string 3 line))) (match-string 4 line))
                      (match-string 2 line)))
            (space (match-string 5 line))
            (task (and (match-string 6 line) "[ ] ")))
        (end-of-line)
        (insert "\n" indent marker space (or task ""))))))

;;; Tables

(defun romacs-markdown--table-at (pos)
  "The table holding POS, as (BEG . END), or nil."
  (pcase-let ((`(,beg . ,end) (romacs-markdown--block-at pos)))
    (seq-some (pcase-lambda (`(,kind ,tbeg ,tend . ,_))
                (and (eq kind 'table) (<= tbeg pos) (<= pos tend) (cons tbeg tend)))
              (romacs-markdown-blocks beg end))))

(defun romacs-markdown-align-table ()
  "Align the columns of the table at point, its cells padded to each
column's widest on the side its delimiter row names."
  (interactive "*" romacs-markdown-mode)
  (pcase-let* ((`(,beg . ,end) (or (romacs-markdown--table-at (point)) (user-error "No table at point")))
               (beg (save-excursion (goto-char beg) (line-beginning-position)))
               (indent (buffer-substring-no-properties beg (save-excursion (goto-char beg) (skip-chars-forward " \t") (point))))
               (source (buffer-substring-no-properties beg end))
               (aligned (or (romacs-markdown--table-align source) (user-error "No table at point")))
               (row (count-lines beg (line-beginning-position)))
               (cell (save-excursion (let ((n 0) (here (point))) (beginning-of-line)
                                       (while (search-forward "|" here t) (setq n (1+ n)))
                                       n))))
    (setq aligned (mapconcat (lambda (line) (if (string-empty-p line) line (concat indent line)))
                             (split-string aligned "\n") "\n"))
    (unless (string-suffix-p "\n" source) (setq aligned (string-remove-suffix "\n" aligned)))
    (unless (equal aligned source)
      (replace-region-contents beg end aligned))
    ;; Point back in its row and cell.
    (goto-char beg)
    (forward-line row)
    (let ((limit (line-end-position)))
      (dotimes (_ cell) (search-forward "|" limit t))
      (skip-chars-forward " " limit))))

;;; Links

(defun romacs-markdown--definition (label)
  "The destination the buffer defines for reference LABEL, or nil."
  (save-excursion
    (goto-char (point-min))
    (let ((case-fold-search t))
      (when (re-search-forward (concat "^ \\{0,3\\}\\[" (regexp-quote label) "\\]:[ \t]*<?\\([^ \t\n>]+\\)") nil t)
        (match-string-no-properties 1)))))

(defun romacs-markdown-follow-link ()
  "Follow the link at point: a URL in a browser, a file in a buffer."
  (interactive nil romacs-markdown-mode)
  (pcase-let* ((`(,beg . ,end) (romacs-markdown--block-at (point)))
               (link (or (seq-find (pcase-lambda (`(,lbeg ,lend . ,_)) (and (<= lbeg (point)) (< (point) lend)))
                                   (romacs-markdown--links beg end))
                         (user-error "No link at point")))
               (`(,lbeg ,lend . ,dest) link))
    (when (string-empty-p dest)
      (let ((text (buffer-substring-no-properties lbeg lend)))
        (setq dest (or (and (string-match "\\]\\[\\([^]]+\\)\\]\\'" text) (romacs-markdown--definition (match-string 1 text)))
                       (and (string-match "\\`!?\\[\\([^]]+\\)\\]" text) (romacs-markdown--definition (match-string 1 text)))
                       (user-error "No definition for this link")))))
    (cond ((string-match-p "\\`[a-zA-Z][a-zA-Z0-9+.-]*:" dest) (browse-url dest))
          ((string-prefix-p "#" dest) (user-error "A link within the file: %s" dest))
          (t (find-file (expand-file-name (car (split-string dest "#")) default-directory))))))

;;; Modes

(defvar-keymap romacs-markdown-mode-map
  :doc "Keys of `romacs-markdown-mode'."
  "M-RET" #'romacs-markdown-insert-item
  "C-c C-c" #'romacs-markdown-align-table
  "C-c C-o" #'romacs-markdown-follow-link
  "C-c C-n" #'outline-next-visible-heading
  "C-c C-p" #'outline-previous-visible-heading
  "C-c C-u" #'outline-up-heading
  "C-c C-f" #'outline-forward-same-level
  "C-c C-b" #'outline-backward-same-level)

;;;###autoload
(define-derived-mode romacs-markdown-mode text-mode "Markdown"
  "Edit Markdown: CommonMark with GFM tables, strikethrough and task lists.
Faces come from `romacs-markdown-faces'; code blocks are coloured as
their language.

\\{romacs-markdown-mode-map}"
  (setq-local font-lock-defaults '(nil t))
  (setq-local font-lock-fontify-region-function #'romacs-markdown--font-lock-region)
  (setq-local outline-regexp "#\\{1,6\\}[ \t]")
  (setq-local outline-search-function #'romacs-markdown--outline-search)
  (setq-local outline-level #'romacs-markdown--outline-level)
  (setq-local imenu-create-index-function #'romacs-markdown--imenu)
  (setq-local comment-start "<!-- ")
  (setq-local comment-end " -->")
  (setq-local comment-start-skip "<!--[ \t]*")
  (setq-local comment-end-skip "[ \t]*-->"))

(defvar-keymap romacs-markdown-view-mode-map
  :doc "Keys of `romacs-markdown-view-mode'."
  :parent romacs-markdown-mode-map
  "q" #'quit-window
  "SPC" #'scroll-up-command
  "DEL" #'scroll-down-command)

;;;###autoload
(define-derived-mode romacs-markdown-view-mode romacs-markdown-mode "Markdown View"
  "Show Markdown read-only, faced as `romacs-markdown-mode' faces it.

\\{romacs-markdown-view-mode-map}"
  (setq-local buffer-read-only t))

;;;###autoload
(dolist (pattern '("\\.md\\'" "\\.markdown\\'" "\\.mdx\\'" "\\.mkd\\'" "\\.mdown\\'"))
  (add-to-list 'auto-mode-alist (cons pattern 'romacs-markdown-mode)))

(provide 'romacs-markdown-mode)
;;; romacs-markdown-mode.el ends here
