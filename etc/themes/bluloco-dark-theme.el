;;; bluloco-dark-theme.el --- Bluloco Dark  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-bluloco-dark by RazikSF, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-bluloco-dark-theme.el; after https://github.com/uloco/theme-bluloco-dark).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme bluloco-dark "Bluloco Dark, after doom-themes."
  :mode dark :family bluloco :style flat
  ((bg "#282c34") (bg-dark "#23252a") (fg "#abb2bf") (fg-dim "#636d83")
   (accent "#10b1fe") (red "#ff6480") (green "#3fc56b") (yellow "#f9c859")
   (blue "#10b1fe") (magenta "#ff78f8") (cyan "#78f8ff") (orange "#ff936a")
   (keyword "#10b1fe") (function "#3fc56b") (type "#ff6480") (constant "#9f7efe")
   (string "#f9c859") (variable "#8bcdef") (builtin "#3fc56b") (comment "#7a808b")
   (doc "#f9c859") (number "#ff78f8")))

;;; bluloco-dark-theme.el ends here
