;;; snazzy-theme.el --- Snazzy  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-snazzy by ar1a, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-snazzy-theme.el; after https://github.com/sindresorhus/hyper-snazzy).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme snazzy "Snazzy, after doom-themes."
  :mode dark :family snazzy :style flat
  ((bg "#282a36") (bg-dark "#242631") (fg "#f9f9f9") (fg-dim "#848688")
   (accent "#57c7ff") (selection "#44464f") (red "#ff5c57") (green "#5af78e")
   (yellow "#f3f99d") (blue "#57c7ff") (magenta "#ff6ac1") (cyan "#9aedfe")
   (orange "#ffb86c") (keyword "#ffb86c") (function "#57c7ff") (type "#9aedfe")
   (constant "#5af78e") (string "#f3f99d") (variable "#ff5c57") (builtin "#ff6ac1")
   (comment "#606580") (doc "#f5fab5") (number "#f3f99d")))

;;; snazzy-theme.el ends here
