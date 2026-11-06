;;; challenger-deep-theme.el --- Challenger Deep  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-challenger-deep by fuxialexander, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-challenger-deep-theme.el; after https://github.com/challenger-deep-theme/vim).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme challenger-deep "Challenger Deep, after doom-themes."
  :mode dark :family challenger-deep :style flat
  ((bg "#1e1c31") (bg-dark "#12111e") (fg "#cbe3e7") (accent "#906cff")
   (selection "#3d4551") (red "#ff8080") (green "#95ffa4") (yellow "#ffe9aa")
   (blue "#91ddff") (magenta "#c991e1") (cyan "#aaffe4") (orange "#ffb378")
   (keyword "#ff8080") (function "#c991e1") (type "#91ddff") (constant "#aaffe4")
   (string "#ffe9aa") (variable "#ffe9aa") (builtin "#c991e1") (comment "#565575")
   (doc "#858fa5") (number "#ffb378")))

;;; challenger-deep-theme.el ends here
