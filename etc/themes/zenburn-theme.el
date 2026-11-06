;;; zenburn-theme.el --- Zenburn  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-zenburn by jsoa, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-zenburn-theme.el; after https://github.com/bbatsov/zenburn-emacs).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme zenburn "Zenburn, after doom-themes."
  :mode dark :family zenburn :style flat
  ((bg "#3f3f3f") (bg-dark "#383838") (fg "#dcdcdc") (accent "#8cd0d3")
   (selection "#2b2b2b") (red "#cc9393") (green "#7f9f7f") (yellow "#f0dfaf")
   (blue "#8cd0d3") (magenta "#dc8cc3") (cyan "#93e0e3") (orange "#dfaf8f")
   (keyword "#f0dfaf") (function "#93e0e3") (type "#7cb8bb") (constant "#bfebbf")
   (string "#cc9393") (variable "#dfaf8f") (builtin "#dcdcdc") (comment "#7f9f7f")
   (doc "#9fc59f") (number "#dcdcdc")))

;;; zenburn-theme.el ends here
