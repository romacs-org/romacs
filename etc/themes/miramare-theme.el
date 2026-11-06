;;; miramare-theme.el --- Miramare  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-miramare by sagittaros, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-miramare-theme.el; after https://github.com/franbach/miramare).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme miramare "Miramare, after doom-themes."
  :mode dark :family miramare :style flat
  ((bg "#2a2426") (bg-dark "#242021") (fg "#e6d6ac") (accent "#d9bb80")
   (selection "#504945") (red "#e68183") (green "#87af87") (yellow "#d9bb80")
   (blue "#89beba") (magenta "#e68183") (cyan "#87c095") (orange "#e39b7b")
   (keyword "#e68183") (function "#87c095") (type "#d9bb80") (constant "#d3a0bc")
   (string "#87af87") (variable "#87c095") (builtin "#e39b7b") (comment "#5b5b5b")
   (doc "#e1d7c0") (number "#d3a0bc")))

;;; miramare-theme.el ends here
