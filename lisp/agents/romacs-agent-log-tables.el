;;; romacs-agent-log-tables.el --- A chat reply's pipe tables as grids -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A reply's top-level pipe tables are drawn as grids once the reply ends
;; (`romacs-markdown-table-grid': columns share the width, cells wrap, a
;; table too wide is drawn as panels), each keeping its source and the
;; width it was drawn at in `romacs-agent-log-table'.  A table drawn at
;; another width than the chat's now is drawn again: as windows resize,
;; and after earlier turns are put above.

;;; Code:

(require 'cl-lib)
(require 'romacs-agent-log)
(require 'romacs-markdown)

(defun romacs-agent-log-tables--width ()
  "The columns a table may take: the narrowest window showing the chat,
less a margin."
  (max 20 (- (romacs-agent-log--rule-width) 2)))

(defun romacs-agent-log-tables--pad-markup (grid)
  "In GRID, pad hidden inline markup to keep the following cells aligned."
  (with-temp-buffer
    (insert grid)
    (romacs-markdown-hide-markup (point-min) (point-max))
    (let ((at (point-min)))
      (while (setq at (text-property-any at (point-max) 'invisible 'romacs-markdown-markup))
        (let ((end (next-single-property-change at 'invisible nil (point-max))))
          (while (< at end)
            (aset grid (- at (point-min)) ?\s)
            (setq at (1+ at)))))))
  grid)

(defun romacs-agent-log-tables--grid (source width)
  "SOURCE, a pipe table, as a grid within WIDTH that keeps SOURCE and
WIDTH in `romacs-agent-log-table', ending in a newline as SOURCE does;
nil when SOURCE holds no table."
  (when-let* ((grid (romacs-markdown-table-grid source width)))
    (unless (string-suffix-p "\n" source) (setq grid (string-remove-suffix "\n" grid)))
    (propertize (romacs-agent-log-tables--pad-markup grid) 'romacs-agent-log-table (list source width (length grid)))))

(defconst romacs-agent-log-tables--drawn
  '(face invisible display fontified composition romacs-agent-log-table)
  "What a table's text carries of its drawing, not of the transcript.")

(defun romacs-agent-log-tables--put (beg end grid)
  "Put GRID in place of BEG..END, with what the transcript marks there:
its entry and round on the first character, the rest throughout."
  (let* ((marks (cl-loop for (key value) on (text-properties-at beg) by #'cddr
                         unless (memq key romacs-agent-log-tables--drawn) append (list key value)))
         (firsts (cl-loop for key in '(romacs-agent-log-entry romacs-agent-log-round)
                          when (plist-member marks key) append (list key (plist-get marks key))))
         (grid (copy-sequence grid)))
    (dolist (key '(romacs-agent-log-entry romacs-agent-log-round)) (cl-remf marks key))
    (add-text-properties 0 (length grid) marks grid)
    (when firsts (add-text-properties 0 1 firsts grid))
    (when (plist-get marks 'romacs-agent-log-dimmed)
      (add-face-text-property 0 (length grid) 'romacs-agent-log-note nil grid))
    (setq grid (romacs-agent-log--sealed grid))
    (romacs-agent-log--writing
      ;; Diff the text so point, mark, and narrowing follow the surviving cells.
      (replace-region-contents beg end grid)
      ;; Matching characters keep old properties in `replace-region-contents'.
      (let ((at 0))
        (while (< at (length grid))
          (let ((next (next-property-change at grid (length grid))))
            (set-text-properties (+ beg at) (+ beg next) (text-properties-at at grid))
            (setq at next)))))
    (length grid)))

(defun romacs-agent-log-tables-draw (start end)
  "Draw each top-level pipe table of the reply START..END as a grid at
the chat's width."
  (when (save-excursion (goto-char start) (search-forward "|" end t))
    (let ((width (romacs-agent-log-tables--width)))
      ;; From the last, so the places of those before stay.
      (dolist (block (reverse (romacs-markdown-blocks start end)))
        (pcase-let ((`(,kind ,beg ,stop . ,plist) block))
          (when (and (eq kind 'table) (eql (plist-get plist :depth) 0))
            (when-let* ((grid (romacs-agent-log-tables--grid (buffer-substring-no-properties beg stop) width)))
              (romacs-agent-log-tables--put beg stop grid))))))))

(defun romacs-agent-log-tables-refit (&rest _)
  "Draw again each table of the chat drawn at another width than its
own now.  They are found by their property: a resize is rare, and no
record of them can go stale."
  (when romacs-agent-log--end
    (save-restriction
      (widen)
      (let ((width (romacs-agent-log-tables--width))
            (at (point-min)))
        (while (setq at (text-property-not-all at romacs-agent-log--end 'romacs-agent-log-table nil))
          (pcase-let* ((`(,source ,drawn ,_) (get-text-property at 'romacs-agent-log-table))
                       (end (next-single-property-change at 'romacs-agent-log-table nil romacs-agent-log--end)))
            (setq at (if-let* (((not (eql drawn width)))
                               (grid (romacs-agent-log-tables--grid source width)))
                         (+ at (romacs-agent-log-tables--put at end grid))
                       end))))))))

(defun romacs-agent-log-tables-source (text)
  "Restore complete grids in TEXT to their original Markdown.
A selection covering only part of a grid keeps the selected characters."
  (let ((at 0) (start 0) pieces)
    (while (setq at (text-property-not-all start (length text) 'romacs-agent-log-table nil text))
      (let ((end (next-single-property-change at 'romacs-agent-log-table text (length text))))
        (pcase-let ((`(,source ,_ ,length) (get-text-property at 'romacs-agent-log-table text)))
          (push (substring text start at) pieces)
          (push (if (= (- end at) length) source (substring text at end)) pieces))
        (setq start end)))
    (if pieces (apply #'concat (nreverse (cons (substring text start) pieces))) text)))

(provide 'romacs-agent-log-tables)
;;; romacs-agent-log-tables.el ends here
