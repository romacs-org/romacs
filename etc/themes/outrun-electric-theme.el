;;; outrun-electric-theme.el --- Outrun Electric  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-outrun-electric by ema2159, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-outrun-electric-theme.el; after https://github.com/samrap/outrun-theme-vscode).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme outrun-electric "Outrun Electric, after doom-themes."
  :mode dark :family outrun-electric :style flat
  ((bg "#0c0a20") (bg-dark "#090819") (fg "#f2f3f7") (fg-dim "#546a90")
   (accent "#1ea8fc") (selection "#1f1147") (red "#e61f44") (green "#a7da1e")
   (yellow "#ffd400") (blue "#1ea8fc") (magenta "#ff2afc") (cyan "#42c6ff")
   (orange "#cf433e") (keyword "#ff2afc") (function "#42c6ff") (type "#ffd400")
   (constant "#df85ff") (string "#7984d1") (variable "#df85ff") (builtin "#1ea8fc")
   (comment "#546a90") (doc "#a875ff") (number "#ffd400")))

;;; outrun-electric-theme.el ends here
