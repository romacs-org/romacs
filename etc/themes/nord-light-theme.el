;;; nord-light-theme.el --- Nord Light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-nord-light by fuxialexander, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-nord-light-theme.el; after https://github.com/arcticicestudio/nord).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme nord-light "Nord Light, after doom-themes."
  :mode light :family nord :style flat
  ((bg "#e5e9f0") (bg-dark "#d8dee9") (fg "#3b4252") (accent "#5d86b6")
   (selection "#b8c5db") (red "#99324b") (green "#4f894c") (yellow "#9a7500")
   (blue "#3b6ea8") (magenta "#97365b") (cyan "#398eac") (orange "#ac4426")
   (keyword "#3b6ea8") (function "#29838d") (type "#9a7500") (constant "#97365b")
   (string "#4f894c") (variable "#842879") (builtin "#29838d") (comment "#8b94a5")
   (doc "#828b9b") (number "#97365b")))

;;; nord-light-theme.el ends here
