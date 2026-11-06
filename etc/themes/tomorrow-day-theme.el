;;; tomorrow-day-theme.el --- Tomorrow Day  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-tomorrow-day by emacswatcher, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-tomorrow-day-theme.el; after https://github.com/ChrisKempson/Tomorrow-Theme).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme tomorrow-day "Tomorrow Day, after doom-themes."
  :mode light :family tomorrow :style flat
  ((bg "#ffffff") (bg-dark "#f2f2f2") (fg "#4d4d4c") (fg-dim "#8e908c")
   (accent "#4271ae") (selection "#e4e4e4") (red "#c82829") (green "#718c00")
   (yellow "#eab700") (blue "#4271ae") (magenta "#c678dd") (cyan "#8abeb7")
   (orange "#f5871f") (keyword "#8959a8") (function "#4271ae") (type "#bb9200")
   (constant "#f5871f") (string "#718c00") (variable "#c82829") (builtin "#4271ae")
   (comment "#8e908c") (doc "#8e908c") (number "#f5871f")))

;;; tomorrow-day-theme.el ends here
