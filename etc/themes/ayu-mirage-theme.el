;;; ayu-mirage-theme.el --- Ayu Mirage  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-ayu-mirage by LoveSponge, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-ayu-mirage-theme.el; after https://github.com/dempfi/ayu).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme ayu-mirage "Ayu Mirage, after doom-themes."
  :mode dark :family ayu :style flat
  ((bg "#1f2430") (bg-dark "#171b24") (fg "#cbccc6") (accent "#ffcc66")
   (selection "#2f3b54") (red "#f28779") (green "#bae67e") (yellow "#ffd580")
   (blue "#73d0ff") (magenta "#d4bfff") (cyan "#5ccfe6") (orange "#ffa759")
   (keyword "#ffa759") (function "#ffd580") (type "#ffe6b3") (constant "#d4bfff")
   (string "#bae67e") (variable "#cbccc6") (comment "#484f5b") (doc "#757b83")
   (number "#ffd580")))

;;; ayu-mirage-theme.el ends here
