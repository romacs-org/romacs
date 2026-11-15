;;; romacs-markdown.el --- Markdown, parsed in Rust -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Romacs's one Markdown: CommonMark with GFM tables, strikethrough and
;; task lists, parsed by pulldown-cmark in Rust (crates/romacs-vm/src/
;; markdown/).  Rust reads the text and writes the faces; this file holds
;; what users customise: the faces, and which face each kind of element
;; gets (`romacs-markdown-faces').
;;
;; - `romacs-markdown-fontify-region' faces a region as one document, in
;;   place.  Called again as text arrives at the region's end (a streamed
;;   reply), it rewrites only the last blocks.
;; - `romacs-markdown-blocks' returns a region's block structure: headings,
;;   code blocks with their language and body, tables, list items, quotes.
;; - `romacs-markdown-string' returns a string's fontified copy.
;; - `romacs-markdown-table-grid' draws a pipe table as a grid within a
;;   width, for text shown rather than edited (an agent's reply).

;;; Code:

(declare-function romacs-markdown--fontify "markdown.rs" (beg end faces &optional from))
(declare-function romacs-markdown--blocks "markdown.rs" (beg end))
(declare-function romacs-markdown--table-grid "markdown.rs" (source width faces))

(defgroup romacs-markdown nil
  "Markdown, parsed in Rust."
  :group 'text
  :prefix "romacs-markdown-")

;;; Faces

(defface romacs-markdown-markup '((t :inherit shadow :slant normal :weight normal))
  "Delimiters, fences, list and quote markers, and table pipes.")
(defface romacs-markdown-heading '((t :inherit bold))
  "Every heading; each level's face inherits it.")
(defface romacs-markdown-heading-1 '((t :inherit (romacs-markdown-heading outline-1)))
  "A level 1 heading.")
(defface romacs-markdown-heading-2 '((t :inherit (romacs-markdown-heading outline-2)))
  "A level 2 heading.")
(defface romacs-markdown-heading-3 '((t :inherit (romacs-markdown-heading outline-3)))
  "A level 3 heading.")
(defface romacs-markdown-heading-4 '((t :inherit (romacs-markdown-heading outline-4)))
  "A level 4 heading.")
(defface romacs-markdown-heading-5 '((t :inherit (romacs-markdown-heading outline-5)))
  "A level 5 heading.")
(defface romacs-markdown-heading-6 '((t :inherit (romacs-markdown-heading outline-6)))
  "A level 6 heading.")
(defface romacs-markdown-emphasis '((t :inherit italic))
  "Emphasis: *text*.")
(defface romacs-markdown-strong '((t :inherit bold))
  "Strong emphasis: **text**.")
(defface romacs-markdown-strike '((t :strike-through t))
  "Strikethrough: ~~text~~.")
(defface romacs-markdown-code-block '((t :inherit fixed-pitch))
  "A code block, fences included.")
(defface romacs-markdown-code '((t :inherit (romacs-markdown-code-block font-lock-constant-face)))
  "An inline code span.")
(defface romacs-markdown-language '((t :inherit font-lock-type-face))
  "A fenced code block's language.")
(defface romacs-markdown-link '((t :inherit link))
  "A link's text, or an autolink.")
(defface romacs-markdown-url '((t :inherit (shadow font-lock-string-face)))
  "A link's destination, or a link reference definition.")
(defface romacs-markdown-image '((t :inherit romacs-markdown-link))
  "An image.")
(defface romacs-markdown-list-marker '((t :inherit (romacs-markdown-markup font-lock-builtin-face)))
  "A list item's bullet or number.")
(defface romacs-markdown-task '((t :inherit font-lock-builtin-face))
  "A task list item's box.")
(defface romacs-markdown-quote '((t :inherit font-lock-doc-face))
  "A block quote.")
(defface romacs-markdown-table '((t :inherit romacs-markdown-code-block))
  "A table.")
(defface romacs-markdown-table-header '((t :inherit bold))
  "A table's header row.")
(defface romacs-markdown-rule '((t :inherit romacs-markdown-markup))
  "A thematic break.")
(defface romacs-markdown-html '((t :inherit font-lock-comment-face))
  "An HTML block or inline HTML.")

(defcustom romacs-markdown-faces
  '((markup . romacs-markdown-markup)
    (heading-1 . romacs-markdown-heading-1) (heading-2 . romacs-markdown-heading-2)
    (heading-3 . romacs-markdown-heading-3) (heading-4 . romacs-markdown-heading-4)
    (heading-5 . romacs-markdown-heading-5) (heading-6 . romacs-markdown-heading-6)
    (emphasis . romacs-markdown-emphasis) (strong . romacs-markdown-strong)
    (strike . romacs-markdown-strike) (code . romacs-markdown-code)
    (code-block . romacs-markdown-code-block) (language . romacs-markdown-language)
    (link . romacs-markdown-link) (url . romacs-markdown-url) (image . romacs-markdown-image)
    (list-marker . romacs-markdown-list-marker) (task . romacs-markdown-task)
    (quote . romacs-markdown-quote) (table . romacs-markdown-table)
    (table-header . romacs-markdown-table-header) (rule . romacs-markdown-rule)
    (html . romacs-markdown-html))
  "The face of each kind of Markdown element.
Where elements nest, the inner one's face comes first in the text's
`face' list.  A kind left out, or given nil, gets no face."
  :type '(alist :key-type (choice (const markup)
                                  (const heading-1) (const heading-2) (const heading-3)
                                  (const heading-4) (const heading-5) (const heading-6)
                                  (const emphasis) (const strong) (const strike)
                                  (const code) (const code-block) (const language)
                                  (const link) (const url) (const image)
                                  (const list-marker) (const task) (const quote)
                                  (const table) (const table-header) (const rule) (const html))
                :value-type (choice face (const :tag "None" nil)))
  :group 'romacs-markdown)

;;; Fontifying and structure

(defun romacs-markdown-fontify-region (beg end &optional from)
  "Face BEG..END of the current buffer as one Markdown document, in place.
Only `face' changes, and no undo is recorded.  With FROM, the text before
FROM has not changed since the last call on this region: the blocks from
the one before FROM's are refaced, the others are left as they are.
Return the position the faces were rewritten from."
  (with-silent-modifications
    (romacs-markdown--fontify beg end romacs-markdown-faces from)))

(defun romacs-markdown-blocks (beg end)
  "The blocks of BEG..END of the current buffer, one Markdown document.
Each is (KIND START END . PLIST), in document order, a nested block
after its container.  KIND is `heading', `code', `table', `item',
`quote', `rule' or `html'.  PLIST has `:depth', the block's nesting in
lists and quotes; a heading's `:level'; a code block's `:language' (a
string, or nil) and `:body' (START . END); an item's `:ordered'."
  (romacs-markdown--blocks beg end))

(defun romacs-markdown-string (text)
  "TEXT, a Markdown document, as a fontified copy."
  (with-temp-buffer
    (insert text)
    (romacs-markdown-fontify-region (point-min) (point-max))
    (buffer-string)))

(defun romacs-markdown-table-grid (source width)
  "SOURCE, a pipe table, as a grid at most WIDTH columns wide, faced.
Columns share the width by what they hold and cells wrap at spaces; a
table too wide for its narrowest columns is drawn as panels of columns,
each with its header.  Nil when SOURCE holds no table."
  (romacs-markdown--table-grid source width romacs-markdown-faces))

(provide 'romacs-markdown)
;;; romacs-markdown.el ends here
