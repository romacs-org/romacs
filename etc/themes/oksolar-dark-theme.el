;;; oksolar-dark-theme.el --- Oksolar Dark  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-oksolar-dark by logc, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-oksolar-dark-theme.el; after https://meat.io/oksolar.json).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme oksolar-dark "Oksolar Dark, after doom-themes."
  :mode dark :family oksolar :style flat
  ((bg "#002d38") (bg-dark "#093946") (fg "#98a8a8") (fg-dim "#657377")
   (accent "#2b90d8") (red "#f23749") (green "#819500") (yellow "#ac8300")
   (blue "#2b90d8") (magenta "#dd459d") (cyan "#259d94") (orange "#d56500")
   (keyword "#819500") (function "#2b90d8") (type "#ac8300") (constant "#dd459d")
   (string "#259d94") (variable "#7d80d1") (builtin "#2b90d8") (comment "#98a8a8")
   (doc "#35a69c") (number "#dd459d")))

;;; oksolar-dark-theme.el ends here
