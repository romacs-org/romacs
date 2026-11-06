;;; manegarm-theme.el --- Manegarm  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-manegarm by kenranunderscore, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-manegarm-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme manegarm "Manegarm, after doom-themes."
  :mode dark :family manegarm :style flat
  ((bg "#1c1408") (bg-dark "#181107") (fg "#5b8512") (fg-dim "#707a6a")
   (accent "#ffbf00") (red "#ff4e00") (green "#7cb518") (yellow "#ffbf00")
   (blue "#0075c4") (magenta "#d72638") (cyan "#898989") (orange "#ff7000")
   (keyword "#ff4e00") (function "#ff7000") (type "#7cb518") (constant "#ff7000")
   (string "#c5ac6b") (variable "#7cb518") (builtin "#ffbf00") (comment "#707a6a")
   (doc "#707a6a") (number "#dbc077")))

;;; manegarm-theme.el ends here
