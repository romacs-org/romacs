;;; monokai-classic-theme.el --- Monokai Classic  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-monokai-classic by ema2159, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-monokai-classic-theme.el; after https://monokai.pro).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme monokai-classic "Monokai Classic, after doom-themes."
  :mode dark :family monokai-classic :style flat
  ((bg "#272822") (bg-dark "#1d1e19") (fg "#f8f8f2") (accent "#fd971f")
   (selection "#4e4e4e") (red "#e74c3c") (green "#a6e22e") (yellow "#e6db74")
   (blue "#268bd2") (magenta "#f92660") (cyan "#66d9ef") (orange "#fd971f")
   (keyword "#f92660") (function "#a6e22e") (type "#66d9ef") (constant "#9c91e4")
   (string "#e6db74") (variable "#f8f8f2") (builtin "#fd971f") (comment "#555556")
   (doc "#7f7f80") (number "#9c91e4")))

;;; monokai-classic-theme.el ends here
