;;; gruvbox-dark-theme.el --- Gruvbox, dark  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Gruvbox by Pavel Pertsev, MIT, https://github.com/morhetz/gruvbox
;; (ef8864bb42bf, colors/gruvbox.vim).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme gruvbox-dark "Gruvbox: retro warmth on dark."
  :mode dark :family gruvbox :style flat
  ((bg "#282828") (bg-dark "#1d2021") (bg-alt "#3c3836") (fg "#ebdbb2")
   (fg-dim "#a89984") (fg-light "#d5c4a1") (fg-bright "#fbf1c7") (muted "#928374")
   (accent "#fabd2f") (selection "#504945") (red "#fb4934") (green "#b8bb26")
   (yellow "#fabd2f") (blue "#83a598") (magenta "#d3869b") (cyan "#8ec07c")
   (orange "#fe8019") (keyword "#fb4934") (function "#b8bb26") (type "#fabd2f")
   (constant "#d3869b") (string "#b8bb26") (variable "#83a598") (builtin "#fe8019")
   (comment "#928374") (ansi-1 "#cc241d") (ansi-2 "#98971a") (ansi-3 "#d79921")
   (ansi-4 "#458588") (ansi-5 "#b16286") (ansi-6 "#689d6a") (ansi-7 "#a89984")
   (ansi-8 "#928374") (ansi-9 "#fb4934") (ansi-10 "#b8bb26") (ansi-11 "#fabd2f")
   (ansi-12 "#83a598") (ansi-13 "#d3869b") (ansi-14 "#8ec07c") (ansi-15 "#ebdbb2")))

;;; gruvbox-dark-theme.el ends here
