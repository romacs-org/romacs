;;; rose-pine-main-theme.el --- Rosé Pine  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Rosé Pine, MIT, https://github.com/rose-pine/neovim
;; (ff483051a47e, lua/rose-pine/palette.lua).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme rose-pine-main "Rosé Pine: soho vibes on dark."
  :mode dark :family rose-pine :style flat
  ((bg "#191724") (bg-dark "#16141f") (bg-alt "#1f1d2e") (fg "#e0def4")
   (fg-dim "#908caa") (fg-light "#908caa") (muted "#6e6a86") (accent "#c4a7e7")
   (selection "#403d52") (red "#eb6f92") (green "#95b1ac") (yellow "#f6c177")
   (blue "#31748f") (magenta "#c4a7e7") (cyan "#9ccfd8") (orange "#ebbcba")
   (keyword "#31748f") (function "#ebbcba") (type "#9ccfd8") (constant "#f6c177")
   (string "#f6c177") (variable "#e0def4") (builtin "#eb6f92") (comment "#6e6a86")))

;;; rose-pine-main-theme.el ends here
