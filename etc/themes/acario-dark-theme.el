;;; acario-dark-theme.el --- Acario Dark  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-acario-dark by gagbo, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-acario-dark-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme acario-dark "Acario Dark, after doom-themes."
  :mode dark :family acario :style flat
  ((bg "#0d0e16") (bg-dark "#040408") (fg "#cedbe5") (fg-dim "#767676")
   (accent "#d85f00") (selection "#1e1e33") (red "#d83441") (green "#79d836")
   (yellow "#d8b941") (blue "#3679d8") (magenta "#8041d8") (cyan "#36d8bd")
   (orange "#d85f00") (keyword "#d83441") (function "#d8b941") (type "#3679d8")
   (constant "#8041d8") (string "#79d836") (variable "#86e7d7") (builtin "#3679d8")
   (comment "#767676") (doc "#9ae168") (number "#d85f00")))

;;; acario-dark-theme.el ends here
