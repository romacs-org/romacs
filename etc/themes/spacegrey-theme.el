;;; spacegrey-theme.el --- Spacegrey  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-spacegrey by teesloane, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-spacegrey-theme.el; after http://kkga.github.io/spacegray/).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme spacegrey "Spacegrey, after doom-themes."
  :mode dark :family spacegrey :style flat
  ((bg "#2b303b") (bg-dark "#232830") (fg "#c0c5ce") (accent "#d08770") (red "#bf616a")
   (green "#a3be8c") (yellow "#ecbe7b") (blue "#8fa1b3") (magenta "#c678dd")
   (cyan "#46d9ff") (orange "#d08770") (keyword "#b48ead") (function "#8fa1b3")
   (type "#ecbe7b") (constant "#d08770") (string "#a3be8c") (variable "#bf616a")
   (builtin "#d08770") (comment "#65737e") (doc "#8b969e") (number "#d08770")))

;;; spacegrey-theme.el ends here
