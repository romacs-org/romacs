;;; ayu-dark-theme.el --- Ayu Dark  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-ayu-dark by ashton, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-ayu-dark-theme.el; after https://github.com/dempfi/ayu).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme ayu-dark "Ayu Dark, after doom-themes."
  :mode dark :family ayu :style flat
  ((bg "#0d1017") (bg-dark "#090c11") (fg "#bfbdb6") (accent "#e6b450")
   (selection "#213145") (red "#f07178") (green "#7fd962") (yellow "#ffb454")
   (blue "#59c2ff") (magenta "#d2a6ff") (cyan "#39bae6") (orange "#ff8f40")
   (keyword "#ff8f40") (function "#ffb454") (type "#e6b673") (constant "#d2a6ff")
   (string "#7fd962") (variable "#bfbdb6") (comment "#373b42") (doc "#696c71")
   (number "#ffb454")))

;;; ayu-dark-theme.el ends here
