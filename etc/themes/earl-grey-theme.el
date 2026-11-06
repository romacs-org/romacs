;;; earl-grey-theme.el --- Earl Grey  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-earl-grey by JuneKelly, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-earl-grey-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme earl-grey "Earl Grey, after doom-themes."
  :mode light :family earl-grey :style flat
  ((bg "#fcfbf9") (bg-dark "#f7f3ee") (fg "#605a52") (fg-dim "#9e9a95")
   (accent "#7686a9") (selection "#f4eaee") (red "#8f5652") (green "#747b4d")
   (yellow "#886a44") (blue "#556995") (magenta "#83577d") (cyan "#477a7b")
   (orange "#886a44") (keyword "#83577d") (function "#605a52") (type "#605a52")
   (constant "#477a7b") (string "#747b4d") (variable "#556995") (builtin "#605a52")
   (comment "#9e9a95") (doc "#9e9a95") (number "#477a7b")))

;;; earl-grey-theme.el ends here
