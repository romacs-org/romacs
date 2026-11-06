;;; sourcerer-theme.el --- Sourcerer  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-sourcerer by fm0xb, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-sourcerer-theme.el; after https://github.com/xero/sourcerer).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme sourcerer "Sourcerer, after doom-themes."
  :mode dark :family sourcerer :style flat
  ((bg "#171717") (bg-dark "#222222") (fg "#c2c2b0") (fg-dim "#686858")
   (accent "#87ceeb") (selection "#32353f") (red "#aa4450") (green "#87875f")
   (yellow "#cc8800") (blue "#87afd7") (magenta "#8787af") (cyan "#87ceeb")
   (orange "#ff9800") (keyword "#87afd7") (function "#faf4c6") (type "#8181a6")
   (constant "#578f8f") (string "#87875f") (variable "#faf4c6") (builtin "#87afd7")
   (comment "#686858") (doc "#5d5d4f") (number "#cc8800")))

;;; sourcerer-theme.el ends here
