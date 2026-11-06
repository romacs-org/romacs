;;; ephemeral-theme.el --- Ephemeral  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-ephemeral by karetsu, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-ephemeral-theme.el; after https://github.com/elenapan/dotfiles).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme ephemeral "Ephemeral, after doom-themes."
  :mode dark :family ephemeral :style flat
  ((bg "#323f4e") (bg-dark "#28323e") (fg "#f8f8f2") (accent "#87dfeb")
   (selection "#1e262d") (red "#f48fb1") (green "#53e2ae") (yellow "#f1fa8c")
   (blue "#92b6f4") (magenta "#bd99ff") (cyan "#79e6f3") (orange "#f2a272")
   (keyword "#f1fa8c") (function "#87dfeb") (type "#79e6f3") (constant "#985eff")
   (string "#f48fb1") (variable "#e7c7f1") (builtin "#f1fa8c") (comment "#56687e")
   (doc "#56687e") (number "#53e2ae")))

;;; ephemeral-theme.el ends here
