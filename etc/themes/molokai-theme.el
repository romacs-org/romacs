;;; molokai-theme.el --- Molokai  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-molokai by Henrik Lissner, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-molokai-theme.el; after https://github.com/tomasr/molokai).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme molokai "Molokai, after doom-themes."
  :mode dark :family molokai :style flat
  ((bg "#1c1e1f") (bg-dark "#222323") (fg "#d6d6d4") (accent "#fd971f")
   (selection "#4e4e4e") (red "#e74c3c") (green "#b6e63e") (yellow "#e2c770")
   (blue "#268bd2") (magenta "#fb2874") (cyan "#66d9ef") (orange "#fd971f")
   (keyword "#fb2874") (function "#b6e63e") (type "#66d9ef") (constant "#fd971f")
   (string "#e2c770") (variable "#fd971f") (builtin "#fd971f") (comment "#555556")
   (doc "#7f7f80") (number "#9c91e4")))

;;; molokai-theme.el ends here
