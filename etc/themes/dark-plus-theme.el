;;; dark-plus-theme.el --- Dark Plus  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-dark+ by ema2159, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-dark+-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme dark-plus "Dark Plus, after doom-themes."
  :mode dark :family dark-plus :style flat
  ((bg "#1e1e1e") (bg-dark "#252526") (fg "#d4d4d4") (fg-dim "#777778")
   (accent "#237ad3") (selection "#113d69") (red "#d16969") (green "#579c4c")
   (yellow "#d7ba7d") (blue "#339cdb") (magenta "#c586c0") (cyan "#85ddff")
   (orange "#db8e73") (keyword "#339cdb") (function "#d9daa2") (type "#35cdaf")
   (constant "#339cdb") (string "#db8e73") (variable "#85ddff") (builtin "#c586c0")
   (comment "#579c4c") (doc "#777778") (number "#b5cea8")))

;;; dark-plus-theme.el ends here
