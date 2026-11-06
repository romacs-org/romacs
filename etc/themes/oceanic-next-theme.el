;;; oceanic-next-theme.el --- Oceanic Next  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-oceanic-next by juanwolf, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-oceanic-next-theme.el; after https://github.com/voronianski/oceanic-next-color-scheme).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme oceanic-next "Oceanic Next, after doom-themes."
  :mode dark :family oceanic-next :style flat
  ((bg "#1b2b34") (bg-dark "#14232d") (fg "#d8dee9") (fg-dim "#a7adba")
   (accent "#fac863") (selection "#4f5b66") (red "#ec5f67") (green "#99c794")
   (yellow "#fac863") (blue "#6699cc") (magenta "#e27e8d") (cyan "#5fb3b3")
   (orange "#f99157") (keyword "#c594c5") (function "#6699cc") (type "#fac863")
   (constant "#f99157") (string "#99c794") (variable "#f99157") (builtin "#ec5f67")
   (comment "#65737e") (doc "#8b969e") (number "#f99157")))

;;; oceanic-next-theme.el ends here
