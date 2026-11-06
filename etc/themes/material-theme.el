;;; material-theme.el --- Material  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-material by tam5, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-material-theme.el; after https://github.com/equinusocio/vsc-material-theme).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme material "Material, after doom-themes."
  :mode dark :family material :style flat
  ((bg "#263238") (bg-dark "#1c262b") (fg "#eeffff") (accent "#c792ea")
   (selection "#171f24") (red "#ff5370") (green "#c3e88d") (yellow "#ffcb6b")
   (blue "#82aaff") (magenta "#c792ea") (cyan "#89ddff") (orange "#f78c6c")
   (keyword "#89ddff") (function "#82aaff") (type "#c792ea") (constant "#f78c6c")
   (string "#c3e88d") (variable "#ffcb6b") (builtin "#82aaff") (comment "#556369")
   (doc "#556369") (number "#f78c6c")))

;;; material-theme.el ends here
