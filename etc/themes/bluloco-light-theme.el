;;; bluloco-light-theme.el --- Bluloco Light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-bluloco-light by RazikSF, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-bluloco-light-theme.el; after https://github.com/uloco/theme-bluloco-light).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme bluloco-light "Bluloco Light, after doom-themes."
  :mode light :family bluloco :style flat
  ((bg "#f9f9f9") (bg-dark "#e7e7e7") (fg "#383a42") (accent "#0098dd")
   (selection "#d5d5d5") (red "#d52753") (green "#23974a") (yellow "#c5a332")
   (blue "#0098dd") (magenta "#ce33c0") (cyan "#33c0ce") (orange "#df631c")
   (keyword "#0098dd") (function "#23974a") (type "#d52753") (constant "#823ff1")
   (string "#c5a332") (variable "#40b8c5") (builtin "#23974a") (comment "#a0a1a7")
   (doc "#c5a332") (number "#ce33c0")))

;;; bluloco-light-theme.el ends here
