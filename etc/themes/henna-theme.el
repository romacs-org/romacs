;;; henna-theme.el --- Henna  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-henna by jsoa, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-henna-theme.el; after https://github.com/httpsterio/vscode-henna).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme henna "Henna, after doom-themes."
  :mode dark :family henna :style flat
  ((bg "#21272e") (bg-dark "#1b1f23") (fg "#f8f8f0") (fg-dim "#737c8c")
   (accent "#e74c3c") (selection "#2e4a54") (red "#e74c3c") (green "#53df83")
   (yellow "#ecbe7b") (blue "#56b5c2") (magenta "#ffb8d1") (cyan "#56b6c2")
   (orange "#e74c3c") (keyword "#1abc9c") (function "#e74c3c") (type "#e74c3c")
   (constant "#1abc9c") (string "#53df83") (variable "#f8f8f0") (builtin "#1abc9c")
   (comment "#606f73") (doc "#606f73") (number "#1abc9c")))

;;; henna-theme.el ends here
