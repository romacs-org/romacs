;;; old-hope-theme.el --- Old Hope  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-old-hope by teesloane, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-old-hope-theme.el; after https://github.com/jesseleite/an-old-hope-syntax-atom).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme old-hope "Old Hope, after doom-themes."
  :mode dark :family old-hope :style flat
  ((bg "#1c1d20") (bg-dark "#151619") (fg "#cbccd1") (accent "#fedd38")
   (selection "#38383b") (red "#ea3d54") (green "#78bd65") (yellow "#fedd38")
   (blue "#4fb3d8") (magenta "#b978ab") (cyan "#4fb3d8") (orange "#ee7b29")
   (keyword "#ea3d54") (function "#fedd38") (type "#ee7b29") (constant "#ee7b29")
   (string "#4fb3d8") (variable "#cbccd1") (builtin "#fedd38") (comment "#5b6268")
   (doc "#7bc5e1") (number "#ee7b29")))

;;; old-hope-theme.el ends here
