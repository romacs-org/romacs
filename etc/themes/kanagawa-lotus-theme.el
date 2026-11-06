;;; kanagawa-lotus-theme.el --- Kanagawa Lotus  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Kanagawa by Tommaso Laurenzi, MIT, https://github.com/rebelot/kanagawa.nvim
;; (bb85e4bfc8d8, lua/kanagawa/colors.lua, themes.lua).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme kanagawa-lotus "Kanagawa Lotus: the same woodblock on warm paper."
  :mode light :family kanagawa :style flat
  ((bg "#f2ecbc") (bg-dark "#e5ddb0") (bg-darker "#dcd5ac") (bg-alt "#e7dba0")
   (fg "#545464") (fg-dim "#716e61") (fg-light "#43436c") (muted "#8a8980")
   (accent "#4d699b") (selection "#c9cbd1") (red "#c84053") (green "#6f894e")
   (yellow "#77713f") (blue "#4d699b") (magenta "#b35b79") (cyan "#597b75")
   (orange "#cc6d00") (bright-red "#d7474b") (bright-green "#6e915f")
   (bright-yellow "#836f4a") (bright-blue "#6693bf") (bright-magenta "#624c83")
   (bright-cyan "#5e857a") (keyword "#624c83") (function "#4d699b") (type "#597b75")
   (constant "#cc6d00") (string "#6f894e") (variable "#545464") (builtin "#c84053")
   (comment "#8a8980")))

;;; kanagawa-lotus-theme.el ends here
