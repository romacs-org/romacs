;;; badger-theme.el --- Badger  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-badger by jsoa, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-badger-theme.el; after https://github.com/ccann/badger-theme/).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme badger "Badger, after doom-themes."
  :mode dark :family badger :style flat
  ((bg "#171717") (bg-dark "#2f2f2f") (fg "#f6f3e8") (accent "#8ac6f2")
   (selection "#434343") (red "#e2434c") (green "#86b187") (yellow "#e0d063")
   (blue "#8ac6f2") (magenta "#e18cbb") (cyan "#65a399") (orange "#ea9847")
   (keyword "#8ac6f2") (function "#ea9847") (type "#c7b299") (constant "#635770")
   (string "#86b187") (variable "#bf93c3") (builtin "#f28b86") (comment "#656868")
   (doc "#8b8d8d") (number "#f6f3e8")))

;;; badger-theme.el ends here
