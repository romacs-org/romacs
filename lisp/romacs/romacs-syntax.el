;;; romacs-syntax.el --- the switches of the syntax summaries -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A long `parse-partial-sexp' composes kept summaries of the text instead
;; of reading all of it (crates/romacs-vm/src/syntax_index.rs,
;; DIVERGENCES.md VM2-SYNTAX-SUMMARIES).  `syntax-ppss' is syntax.el's: its
;; long parses from the states it caches are the ones served.

;;; Code:

(defvar romacs-inhibit-syntax-summaries nil
  "Non-nil makes `parse-partial-sexp' scan every character, as GNU's does.
`romacs-syntax-summaries-stats' shows what the summaries served.")

(defvar romacs-syntax-summaries-check nil
  "Non-nil makes each `parse-partial-sexp' the summaries serve scan as well,
and signal an error where the two differ.")

(provide 'romacs-syntax)
;;; romacs-syntax.el ends here
