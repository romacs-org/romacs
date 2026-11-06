;;; peacock-theme.el --- Peacock  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-peacock by teesloane, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-peacock-theme.el; after https://github.com/daylerees/colour-schemes/blob/master/emacs/peacock-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme peacock "Peacock, after doom-themes."
  :mode dark :family peacock :style flat
  ((bg "#2b2a27") (bg-dark "#1f1e1d") (fg "#ede0ce") (accent "#ff5d38")
   (selection "#403f3e") (red "#ff5d38") (green "#98be65") (yellow "#bcd42a")
   (blue "#51afef") (magenta "#c678dd") (cyan "#46d9ff") (orange "#cb4b16")
   (keyword "#26a6a6") (function "#bcd42a") (type "#f8f8f0") (constant "#ff5d38")
   (string "#bcd42a") (variable "#f8f8f0") (builtin "#ff5d38") (comment "#5b6268")
   (doc "#83898d") (number "#ff5d38")))

;;; peacock-theme.el ends here
