;;; fairy-floss-theme.el --- Fairy Floss  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-fairy-floss by ema2159, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-fairy-floss-theme.el; after https://github.com/sailorhg/fairyfloss).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme fairy-floss "Fairy Floss, after doom-themes."
  :mode dark :family fairy-floss :style flat
  ((bg "#5a5475") (bg-dark "#343145") (fg "#f8f8f0") (accent "#c5a3ff")
   (selection "#464258") (red "#cc6666") (green "#c2ffdf") (yellow "#ffea00")
   (blue "#55b3cc") (magenta "#ffb8d1") (cyan "#96cbfe") (orange "#e6c000")
   (keyword "#96cbfe") (function "#c2ffdf") (type "#96cbfe") (constant "#c5a3ff")
   (string "#ffea00") (variable "#ffb8d1") (builtin "#55b3cc") (comment "#e6c000")
   (doc "#c5a3ff") (number "#c5a3ff")))

;;; fairy-floss-theme.el ends here
