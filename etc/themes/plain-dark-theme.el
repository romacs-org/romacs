;;; plain-dark-theme.el --- Plain Dark  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-plain-dark by das-s, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-plain-dark-theme.el; after https://github.com/gko/plain/).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme plain-dark "Plain Dark, after doom-themes."
  :mode dark :family plain :style flat
  ((bg "#222222") (bg-dark "#434343") (fg "#d7d5d1") (fg-dim "#d7d5d1")
   (accent "#bbbbbb") (selection "#0e0c0a") (red "#d7d5d1") (green "#d7d5d1")
   (yellow "#d7d5d1") (blue "#d7d5d1") (magenta "#d7d5d1") (cyan "#d7d5d1")
   (orange "#d7d5d1") (keyword "#d7d5d1") (function "#d7d5d1") (type "#d7d5d1")
   (constant "#838083") (string "#838083") (variable "#838083") (builtin "#838083")
   (comment "#545053") (doc "#545053") (number "#838083")))

;;; plain-dark-theme.el ends here
