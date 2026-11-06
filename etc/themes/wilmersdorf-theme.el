;;; wilmersdorf-theme.el --- Wilmersdorf  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-wilmersdorf by ema2159, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-wilmersdorf-theme.el; after https://github.com/ianpan870102/wilmersdorf-emacs-theme).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme wilmersdorf "Wilmersdorf, after doom-themes."
  :mode dark :family wilmersdorf :style flat
  ((bg "#282b33") (bg-dark "#1f2024") (fg "#c6c6c6") (accent "#819cd6")
   (selection "#41454b") (red "#e1c1ee") (green "#5b94ab") (yellow "#cfcf9c")
   (blue "#819cd6") (magenta "#a6c1e0") (cyan "#7289bc") (orange "#a6c1e0")
   (keyword "#819cd6") (function "#7ebebd") (type "#b0a2e7") (constant "#a6c1e0")
   (string "#5b94ab") (variable "#a6c1e0") (builtin "#7ebebd") (comment "#6e7899")
   (doc "#9299b2") (number "#a6c1e0")))

;;; wilmersdorf-theme.el ends here
