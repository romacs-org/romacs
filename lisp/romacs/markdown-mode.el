;;; markdown-mode.el --- markdown-mode's names for romacs-markdown-mode -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Romacs edits Markdown with `romacs-markdown-mode' (romacs-markdown-mode.el).
;; Code that names jrblevin's markdown-mode, its modes, hook and map, or
;; requires it, gets Romacs's: so (require 'markdown-mode) succeeds and
;; `markdown-mode-hook' runs.  markdown-mode's own functions are not here; a
;; package that calls them installs markdown-mode, whose files load ahead
;; of these.  ledger/divergences/MD1.md.

;;; Code:

(require 'romacs-markdown-mode)

;;;###autoload
(defalias 'markdown-mode #'romacs-markdown-mode)
;;;###autoload
(defalias 'gfm-mode #'romacs-markdown-mode)
;;;###autoload
(defalias 'markdown-view-mode #'romacs-markdown-view-mode)
;;;###autoload
(defalias 'gfm-view-mode #'romacs-markdown-view-mode)

(defvaralias 'markdown-mode-hook 'romacs-markdown-mode-hook)
(defvaralias 'markdown-mode-map 'romacs-markdown-mode-map)

(defvar markdown-fontify-code-blocks-natively t
  "Ignored: `romacs-markdown-mode' always colours code blocks by language.")

(provide 'markdown-mode)
;;; markdown-mode.el ends here
