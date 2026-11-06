;;; feather-light-theme.el --- Feather Light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-feather-light by Lena SAVY-LARIGALDIE, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-feather-light-theme.el; after https://github.com/Plunne/doom-feather-theme).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme feather-light "Feather Light, after doom-themes."
  :mode light :family feather :style flat
  ((bg "#f0edf4") (bg-dark "#e8e4ee") (fg "#4a3b5e") (accent "#9783b1")
   (selection "#d0cdd6") (red "#dc322f") (green "#5f8700") (yellow "#a07000")
   (blue "#007daf") (magenta "#f4649b") (cyan "#008ea1") (orange "#d75f00")
   (keyword "#875faf") (function "#007daf") (type "#f4649b") (constant "#008070")
   (string "#5f8700") (variable "#513969") (builtin "#007daf") (comment "#b4a5c7")
   (doc "#998ca9") (number "#008070")))

;;; feather-light-theme.el ends here
