;;; ayu-light-theme.el --- Ayu Light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-ayu-light by LoveSponge, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-ayu-light-theme.el; after https://github.com/dempfi/ayu).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme ayu-light "Ayu Light, after doom-themes."
  :mode light :family ayu :style flat
  ((bg "#fafafa") (bg-dark "#fafafa") (fg "#575f66") (accent "#ff9940")
   (selection "#bce1f3") (red "#f07171") (green "#86b300") (yellow "#f2ae49")
   (blue "#399ee6") (magenta "#a37acc") (cyan "#55b4d4") (orange "#fa8d3e")
   (keyword "#fa8d3e") (function "#f2ae49") (type "#e6ba7e") (constant "#a37acc")
   (string "#86b300") (variable "#575f66") (builtin "#f2ae49") (comment "#c4c7c9")
   (doc "#d2d5d6") (number "#f2ae49")))

;;; ayu-light-theme.el ends here
