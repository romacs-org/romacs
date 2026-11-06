;;; rose-pine-dawn-theme.el --- Rosé Pine Dawn  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Rosé Pine, MIT, https://github.com/rose-pine/neovim
;; (ff483051a47e, lua/rose-pine/palette.lua).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme rose-pine-dawn "Rosé Pine Dawn: the same garden at first light."
  :mode light :family rose-pine :style flat
  ((bg "#faf4ed") (bg-dark "#f8f0e7") (bg-alt "#f2e9e1") (fg "#464261")
   (fg-dim "#797593") (fg-light "#797593") (muted "#9893a5") (accent "#907aa9")
   (selection "#dfdad9") (red "#b4637a") (green "#6d8f89") (yellow "#ea9d34")
   (blue "#286983") (magenta "#907aa9") (cyan "#56949f") (orange "#d7827e")
   (keyword "#286983") (function "#d7827e") (type "#56949f") (constant "#ea9d34")
   (string "#ea9d34") (variable "#464261") (builtin "#b4637a") (comment "#9893a5")))

;;; rose-pine-dawn-theme.el ends here
