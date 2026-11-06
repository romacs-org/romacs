;;; laserwave-theme.el --- Laserwave  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-laserwave by hyakt, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-laserwave-theme.el; after https://github.com/Jaredk3nt/laserwave).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme laserwave "Laserwave, after doom-themes."
  :mode dark :family laserwave :style flat
  ((bg "#27212e") (bg-dark "#1b1720") (fg "#ffffff") (accent "#eb64b9")
   (selection "#4e2e49") (red "#964c7b") (green "#74dfc4") (yellow "#ffe261")
   (blue "#40b4c4") (magenta "#eb64b9") (cyan "#b4dce7") (orange "#ffb85b")
   (keyword "#40b4c4") (function "#eb64b9") (type "#ffe261") (constant "#b381c5")
   (string "#b4dce7") (variable "#ffffff") (builtin "#eb64b9") (comment "#91889b")
   (doc "#919ea7") (number "#ffb85b")))

;;; laserwave-theme.el ends here
