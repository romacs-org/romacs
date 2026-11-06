;;; tokyonight-night-theme.el --- Tokyo Night, the night variant  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Tokyo Night by Folke Lemaitre, Apache-2.0, https://github.com/folke/tokyonight.nvim
;; (cdc07ac78467, extras/lua/tokyonight_night.lua).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme tokyonight-night "Tokyo Night: deep indigo with neon accents."
  :mode dark :family tokyonight :style flat
  ((bg "#1a1b26") (bg-dark "#16161e") (bg-darker "#0c0e14") (bg-alt "#292e42")
   (fg "#c0caf5") (fg-dim "#737aa2") (fg-light "#a9b1d6") (muted "#545c7e")
   (accent "#7aa2f7") (selection "#283457") (red "#f7768e") (green "#9ece6a")
   (yellow "#e0af68") (blue "#7aa2f7") (magenta "#bb9af7") (cyan "#7dcfff")
   (orange "#ff9e64") (bright-red "#ff899d") (bright-green "#9fe044")
   (bright-yellow "#faba4a") (bright-blue "#8db0ff") (bright-magenta "#c7a9ff")
   (bright-cyan "#a4daff") (keyword "#9d7cd8") (function "#7aa2f7") (type "#2ac3de")
   (constant "#ff9e64") (string "#9ece6a") (variable "#c0caf5") (builtin "#2ac3de")
   (comment "#565f89") (ansi-0 "#15161e") (ansi-7 "#a9b1d6") (ansi-8 "#414868")
   (ansi-15 "#c0caf5")))

;;; tokyonight-night-theme.el ends here
