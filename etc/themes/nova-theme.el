;;; nova-theme.el --- Nova  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-nova by bigardone, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-nova-theme.el; after https://github.com/trevordmiller/nova-colors).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme nova "Nova, after doom-themes."
  :mode dark :family nova :style flat
  ((bg "#3c4c55") (bg-dark "#44545d") (fg "#c5d4dd") (fg-dim "#899ba6")
   (accent "#7fc1ca") (red "#df8c8c") (green "#a8ce93") (yellow "#dada93")
   (blue "#83afe5") (magenta "#d18ec2") (cyan "#7fc1ca") (orange "#f2c38f")
   (keyword "#9a93e1") (function "#83afe5") (type "#a8ce93") (constant "#7fc1ca")
   (string "#7fc1ca") (variable "#df8c8c") (builtin "#83afe5") (comment "#899ba6")
   (doc "#94a4ae") (number "#7fc1ca")))

;;; nova-theme.el ends here
