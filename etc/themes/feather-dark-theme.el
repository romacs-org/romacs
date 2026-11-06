;;; feather-dark-theme.el --- Feather Dark  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-feather-dark by Lena SAVY-LARIGALDIE, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-feather-dark-theme.el; after https://github.com/Plunne/doom-feather-theme).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme feather-dark "Feather Dark, after doom-themes."
  :mode dark :family feather :style flat
  ((bg "#251d2f") (bg-dark "#1a1521") (fg "#c4b8d3") (accent "#9783b1")
   (selection "#3c3842") (red "#ff6c6b") (green "#98be65") (yellow "#ecbe7b")
   (blue "#51afef") (magenta "#7a619a") (cyan "#7dcfff") (orange "#da8548")
   (keyword "#9783b1") (function "#51afef") (type "#1abc9c") (constant "#da8548")
   (string "#98be65") (variable "#b1e2ff") (builtin "#7a619a") (comment "#503f65")
   (doc "#7b6f8b") (number "#da8548")))

;;; feather-dark-theme.el ends here
