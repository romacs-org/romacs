;;; homage-black-theme.el --- Homage Black  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-homage-black by mskorzhinskiy, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-homage-black-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme homage-black "Homage Black, after doom-themes."
  :mode dark :family homage :style flat
  ((bg "#000000") (bg-dark "#000000") (fg "#bbc2cf") (fg-dim "#5b6268")
   (accent "#0170bf") (selection "#00365a") (red "#ff6c6b") (green "#98be65")
   (yellow "#ecbe7b") (blue "#0170bf") (magenta "#c678dd") (cyan "#46d9ff")
   (orange "#b4916d") (keyword "#bbc2cf") (function "#0170bf") (type "#bbc2cf")
   (constant "#bbc2cf") (string "#b4916d") (variable "#bbc2cf") (builtin "#bbc2cf")
   (comment "#98be65") (doc "#81a155") (number "#b4916d")))

;;; homage-black-theme.el ends here
