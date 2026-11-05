;;; solarized-dark-high-contrast-theme.el --- Solarized Dark High Contrast  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-solarized-dark-high-contrast by jmorag, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-solarized-dark-high-contrast-theme.el; after https://github.com/bbatsov/solarized-emacs).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme solarized-dark-high-contrast "Solarized Dark High Contrast, after doom-themes."
  :mode dark :family solarized :style flat
  ((bg "#002732") (bg-dark "#00212b") (fg "#8d9fa1") (fg-dim "#56697a")
   (accent "#3c98e0") (selection "#01323d") (red "#ec423a") (green "#93a61a")
   (yellow "#c49619") (blue "#3c98e0") (magenta "#e2468f") (cyan "#3cafa5")
   (orange "#db5823") (keyword "#93a61a") (function "#3c98e0") (type "#c49619")
   (constant "#3c98e0") (string "#93a61a") (variable "#8d9fa1") (builtin "#3c98e0")
   (comment "#62787f") (doc "#35a69c") (number "#7a7ed2")))

;;; solarized-dark-high-contrast-theme.el ends here
