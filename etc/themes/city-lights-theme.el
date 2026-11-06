;;; city-lights-theme.el --- City Lights  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-city-lights by fuxialexander, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-city-lights-theme.el; after https://citylights.xyz).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme city-lights "City Lights, after doom-themes."
  :mode dark :family city-lights :style flat
  ((bg "#1d252c") (bg-dark "#181e24") (fg "#a0b3c5") (accent "#5ec4ff")
   (selection "#28323b") (red "#d95468") (green "#8bd49c") (yellow "#ebbf83")
   (blue "#5ec4ff") (magenta "#e27e8d") (cyan "#70e1e8") (orange "#d98e48")
   (keyword "#5ec4ff") (function "#33ced8") (type "#ebbf83") (constant "#e27e8d")
   (string "#539afc") (variable "#718ca1") (builtin "#5ec4ff") (comment "#41505e")
   (doc "#707b86") (number "#e27e8d")))

;;; city-lights-theme.el ends here
