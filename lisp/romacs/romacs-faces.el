;;; romacs-faces.el --- The faces of Romacs's chrome  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The faces the panes, layers, toasts, the landing and the completion
;; panel are drawn in.  None carries a colour of its own: a theme sets
;; them (lisp/romacs/romacs-theme.el), and without one they inherit standard
;; faces or are derived from them (romacs-chrome.el
;; `romacs-chrome-derived-faces').

;;; Code:

(defgroup romacs-faces nil
  "The faces of Romacs's chrome."
  :group 'faces)

(defface romacs-layer-glow '((t))
  "The colour a raised layer glows in, the theme's accent: the focus edge,
a layer's glow, completion matches.")

(defface romacs-mode-line-mark '((t))
  "Romacs's R in the status bar: the mark's red where it shows on the
bar (`romacs-chrome-derived-faces'), else the bar's own foreground.")

(defface romacs-pane-focus '((t :inherit romacs-layer-glow))
  "The selected pane's edge: its foreground.")

(defface romacs-pane-border '((t))
  "The hairline round every other pane: its foreground.")

(defface romacs-layer-glass '((t))
  "A glass layer's background.")

(defface romacs-layer-toast '((t :inherit romacs-layer-glass))
  "A message toast's text and background.")

(defface romacs-layer-toast-error '((t :inherit error))
  "An error toast: its foreground is the toast's glow.")

(defface romacs-layer-selection '((t :inherit region))
  "The band a layer's selected row is drawn on.")

(defface romacs-landing-mark '((t :inherit romacs-layer-glow))
  "The landing's mark: its key light.")

(defface romacs-landing-word '((t :inherit default))
  "The landing's wordmark.")

(defface romacs-complete-current '((t :extend t))
  "Face used to highlight the currently selected candidate.")

(defface romacs-complete-match '((t :inherit romacs-layer-glow))
  "Face of the characters a completion style matched in a candidate,
in place of the style's own (`romacs-fzf-match', `completions-common-part',
orderless's).")

(defface romacs-complete-prompt '((t :inherit shadow))
  "Face of a completing prompt's label and count in the panel: the input
is what the panel shows first.")

(provide 'romacs-faces)
;;; romacs-faces.el ends here
