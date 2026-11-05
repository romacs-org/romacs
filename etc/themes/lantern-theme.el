;;; lantern-theme.el --- Lantern  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-lantern by paladhammika, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-lantern-theme.el; after https://github.com/Gitleptune/lantern-theme).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme lantern "Lantern, after doom-themes."
  :mode dark :family lantern :style flat
  ((bg "#261b17") (bg-dark "#291812") (fg "#e4cbb3") (accent "#aa6a1f")
   (selection "#532f28") (red "#cc241d") (green "#98971a") (yellow "#d79921")
   (blue "#66a4a4") (magenta "#d3869b") (cyan "#46d9ff") (orange "#e35b15")
   (keyword "#66a4a4") (function "#e35b15") (type "#d79921") (constant "#805b87")
   (string "#98971a") (variable "#e4b6c3") (builtin "#e35b15") (comment "#c18e76")
   (doc "#7f7370") (number "#d79921")))

;;; lantern-theme.el ends here
