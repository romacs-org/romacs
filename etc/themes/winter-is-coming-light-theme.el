;;; winter-is-coming-light-theme.el --- Winter Is Coming Light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-winter-is-coming-light by Philip Arvidsson, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-winter-is-coming-light-theme.el; after https://github.com/johnpapa/vscode-winteriscoming).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme winter-is-coming-light "Winter Is Coming Light, after doom-themes."
  :mode light :family winter-is-coming :style flat
  ((bg "#ffffff") (bg-dark "#eff2ef") (fg "#1857a4") (fg-dim "#8195b0")
   (accent "#2f86d2") (red "#ef5350") (green "#357b42") (yellow "#f7ecb5")
   (blue "#2f86d2") (magenta "#dc3eb7") (cyan "#219fd5") (orange "#fa841d")
   (keyword "#219fd5") (function "#dc3eb7") (type "#dc3eb7") (constant "#2f86d2")
   (string "#a44185") (variable "#1857a4") (builtin "#dc3eb7") (comment "#8195b0")
   (doc "#60a8a1") (number "#ef5350")))

;;; winter-is-coming-light-theme.el ends here
