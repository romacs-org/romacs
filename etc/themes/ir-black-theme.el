;;; ir-black-theme.el --- Ir Black  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-ir-black by Kevin Kainan Li, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-ir-black-theme.el; after https://github.com/twerth/ir_black).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme ir-black "Ir Black, after doom-themes."
  :mode dark :family ir-black :style flat
  ((bg "#000000") (bg-dark "#121212") (fg "#f6f3e8") (accent "#96cbfe")
   (selection "#353535") (red "#ff6c60") (green "#a8ff60") (yellow "#ffffb6")
   (blue "#96cbfe") (magenta "#ff73fd") (cyan "#c6c5fe") (orange "#e9c062")
   (keyword "#96cbfe") (function "#ffd2a7") (type "#ffffb6") (constant "#99cc99")
   (string "#a8ff60") (variable "#ffabfd") (builtin "#ff73fd") (comment "#5b6268")
   (doc "#83898d") (number "#ff73fd")))

;;; ir-black-theme.el ends here
