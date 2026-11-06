;;; oksolar-light-theme.el --- Oksolar Light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-oksolar-light by logc, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-oksolar-light-theme.el; after https://meat.io/oksolar.json).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme oksolar-light "Oksolar Light, after doom-themes."
  :mode light :family oksolar :style flat
  ((bg "#fbf7ef") (bg-dark "#f1e9d2") (fg "#657377") (accent "#2b90d8") (red "#f23749")
   (green "#819500") (yellow "#ac8300") (blue "#2b90d8") (magenta "#dd459d")
   (cyan "#259d94") (orange "#d56500") (keyword "#819500") (function "#dd459d")
   (type "#ac8300") (constant "#7d80d1") (string "#259d94") (variable "#2b90d8")
   (builtin "#dd459d") (comment "#657377") (doc "#35a69c") (number "#7d80d1")))

;;; oksolar-light-theme.el ends here
