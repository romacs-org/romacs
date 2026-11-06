;;; acario-light-theme.el --- Acario Light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-acario-light by gagbo, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-acario-light-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme acario-light "Acario Light, after doom-themes."
  :mode light :family acario :style flat
  ((bg "#f5f5f9") (bg-dark "#e9e9f2") (fg "#0f1019") (fg-dim "#4e4e4e")
   (accent "#009b7c") (selection "#c0ccd0") (red "#d70000") (green "#005f00")
   (yellow "#af8700") (blue "#1f55a0") (magenta "#af005f") (cyan "#007687")
   (orange "#d75f00") (keyword "#d70000") (function "#af8700") (type "#1f55a0")
   (constant "#af005f") (string "#005f00") (variable "#004651") (builtin "#1f55a0")
   (comment "#4e4e4e") (doc "#004700") (number "#d75f00")))

;;; acario-light-theme.el ends here
