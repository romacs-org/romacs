;;; opera-light-theme.el --- Opera Light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-opera-light by jwintz, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-opera-light-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme opera-light "Opera Light, after doom-themes."
  :mode light :family opera-light :style flat
  ((bg "#fafafa") (bg-dark "#eeeeee") (fg "#2a2a2a") (accent "#3b6ea8")
   (selection "#bdbdbd") (red "#99324b") (green "#4f894c") (yellow "#9a7500")
   (blue "#3b6ea8") (magenta "#97365b") (cyan "#398eac") (orange "#ac4426")
   (keyword "#3b6ea8") (function "#29838d") (type "#9a7500") (constant "#97365b")
   (string "#4f894c") (variable "#cb9aad") (builtin "#29838d") (comment "#b1b1b1")
   (doc "#b6b6b6") (number "#97365b")))

;;; opera-light-theme.el ends here
