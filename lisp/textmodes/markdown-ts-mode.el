;;; markdown-ts-mode.el --- markdown-ts-mode's names for romacs-markdown-mode -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Romacs edits Markdown with `romacs-markdown-mode', not GNU's
;; tree-sitter markdown-ts-mode, which it does not ship.  GNU's autoloads
;; name this file for markdown-ts-mode's commands: they get Romacs's modes.
;; ledger/divergences/MD1.md.

;;; Code:

(require 'romacs-markdown-mode)

;;;###autoload
(defalias 'markdown-ts-mode #'romacs-markdown-mode)
;;;###autoload
(defalias 'markdown-ts-mode-maybe #'romacs-markdown-mode)
;;;###autoload
(defalias 'markdown-ts-view-mode #'romacs-markdown-view-mode)

(provide 'markdown-ts-mode)
;;; markdown-ts-mode.el ends here
