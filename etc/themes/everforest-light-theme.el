;;; everforest-light-theme.el --- Everforest, light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Everforest by sainnhe, MIT, https://github.com/sainnhe/everforest
;; (85a86eb62409, autoload/everforest.vim, medium contrast).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme everforest-light "Everforest: quiet greens on paper."
  :mode light :family everforest :style flat
  ((bg "#fdf6e3") (bg-dark "#efebd4") (bg-alt "#f4f0d9") (fg "#5c6a72")
   (fg-dim "#829181") (fg-light "#939f91") (muted "#a6b0a0") (accent "#8da101")
   (selection "#eaedc8") (red "#f85552") (green "#8da101") (yellow "#dfa000")
   (blue "#3a94c5") (magenta "#df69ba") (cyan "#35a77c") (orange "#f57d26")
   (keyword "#f85552") (function "#8da101") (type "#dfa000") (constant "#df69ba")
   (string "#35a77c") (variable "#3a94c5") (builtin "#f57d26") (comment "#939f91")))

;;; everforest-light-theme.el ends here
