;;; nord-aurora-theme.el --- Nord Aurora  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-nord-aurora by MoskitoHero, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-nord-aurora-theme.el; after https://www.nordtheme.com).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme nord-aurora "Nord Aurora, after doom-themes."
  :mode dark :family nord-aurora :style flat
  ((bg "#2e3440") (bg-dark "#272c36") (fg "#eceff4") (accent "#9099ab")
   (selection "#434c5e") (red "#bf616a") (green "#a3be8c") (yellow "#ebcb8b")
   (blue "#81a1c1") (magenta "#b48ead") (cyan "#88c0d0") (orange "#d08770")
   (keyword "#81a1c1") (function "#ebcb8b") (type "#a3be8c") (constant "#81a1c1")
   (string "#8fbcbb") (variable "#88c0d0") (builtin "#b48ead") (comment "#9099ab")
   (doc "#d8dee9") (number "#d08770")))

;;; nord-aurora-theme.el ends here
