;;; shades-of-purple-theme.el --- Shades Of Purple  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-shades-of-purple by jwbaldwin, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-shades-of-purple-theme.el; after https://github.com/jwbaldwin/spacemacs-shades-of-purple).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme shades-of-purple "Shades Of Purple, after doom-themes."
  :mode dark :family shades-of-purple :style flat
  ((bg "#2d2b55") (bg-dark "#1e1e3f") (fg "#e3e9fa") (accent "#82aaff") (red "#ff000d")
   (green "#3ad900") (yellow "#fad000") (blue "#82aaff") (magenta "#ff9d00")
   (cyan "#ff628c") (orange "#ff9d00") (keyword "#ff9d00") (function "#fad726")
   (type "#3ad900") (constant "#fb94ff") (string "#a5ff90") (variable "#37fea1")
   (builtin "#ff9d00") (comment "#b362ff") (doc "#c589ff") (number "#ff9d00")))

;;; shades-of-purple-theme.el ends here
