;;; flatwhite-theme.el --- Flatwhite  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-flatwhite by JuneKelly, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-flatwhite-theme.el; after https://github.com/biletskyy/flatwhite-syntax).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme flatwhite "Flatwhite, after doom-themes."
  :mode light :family flatwhite :style flat
  ((bg "#f7f3ee") (bg-dark "#f1ece4") (fg "#605a52") (accent "#7382a0")
   (selection "#d8d4cd") (red "#955f5f") (green "#81895d") (yellow "#957f5f")
   (blue "#7382a0") (magenta "#9c739c") (cyan "#5f8c7d") (orange "#957f5f")
   (keyword "#955f5f") (function "#9c739c") (type "#957f5f") (constant "#9c739c")
   (string "#81895d") (variable "#634963") (builtin "#9c739c") (comment "#b9a992")
   (doc "#9d8f7c") (number "#957f5f")))

;;; flatwhite-theme.el ends here
