;;; romacs-theme-rum.el --- Rum's definition, shared by `rum' and `default'  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Rum, as the reference session configures it on Modus 5.3.0, as data:
;; the :mode, :modus and :palette properties `romacs-theme-define' takes.
;; :modus is Rum's own definition (its base colours, its warm preference,
;; the vivendi core and its palette overrides), from which Modus's face
;; table makes every face (lisp/romacs/romacs-theme-modus.el); Romacs's own
;; faces take :palette, the Rum palette as base and pinned slots
;; (lisp/romacs/romacs-theme.el), which a launch without Custom uses for every
;; face.  `rum' (rum-theme.el) is exactly this look, which
;; crates/romacs-vm-gui/tests/romacs-theme-rum-test.el compares with
;; GNU's; the stock theme `default' starts from it.

;;; Code:

(defconst romacs-theme-rum
  (list :mode 'dark
        :modus
        '(:base
          ((bg-main "#000000") (fg-main "#d4c5a0") (fg-dim "#756b59") (red "#c97054")
           (red-warmer "#ff6b55") (red-cooler "#d8796f") (red-faint "#c98f7d") (red-intense "#ff5f59")
           (green "#9cbd6f") (green-warmer "#b3a35c") (green-cooler "#78a68b") (green-faint "#a6bd95")
           (green-intense "#44df44") (yellow "#d48a20") (yellow-warmer "#e89e30") (yellow-cooler "#c8b890")
           (yellow-faint "#d2b580") (yellow-intense "#f0b84b") (gold "#d48a20") (fg-link-rum "#4cc9f0")
           (blue "#5b9bd5") (blue-warmer "#9099d9") (blue-cooler "#72b6e4") (blue-faint "#7ea8bd")
           (blue-intense "#72b6e4") (indigo "#9099d9") (magenta "#c28fea") (magenta-warmer "#d0a3e8")
           (magenta-cooler "#b395db") (magenta-faint "#aa91c2") (magenta-intense "#c28fea") (cyan "#00a6a6")
           (cyan-warmer "#4fb8b4") (cyan-cooler "#00b8ad") (cyan-faint "#8bbcb2") (cyan-intense "#00d3d0")
           (olive "#aacb75") (bg-lavender "#38325c") (bg-mode-line-rum "#50269a") (bg-magenta-intense "#7030af")
           (bg-magenta-nuanced "#2f0c3f") (bg-term-white "#a6a6a6"))
          :preference warm
          :core romacs-theme-modus-vivendi-palette
          ;; Rum's palette overrides, verbatim.
          :overrides
          ((comment fg-dim)
           (mail-cite-1 green)
           (fg-link fg-link-rum)
           (underline-link fg-link-rum)
           (fg-line-number-active cyan-intense)
           (fg-heading-1 red-intense)
           (bg-line-number-inactive unspecified)
           (bg-line-number-active unspecified)
           (bg-paren-match bg-magenta-intense)
           (underline-paren-match fg-main)
           (underline-err red-intense)
           (underline-warning yellow-faint)
           (underline-note cyan-faint)
           (string olive)
           (border-mode-line-active unspecified)
           (border-mode-line-inactive unspecified)
           (bg-mode-line-active bg-mode-line-rum)
           (cursor indigo)
           (prose-todo green-intense)
           (prose-done bg-term-white)
           (fg-prompt yellow-faint)
           (bg-prose-block-contents bg-magenta-nuanced)
           (bg-prose-block-delimiter bg-lavender)
           (fg-prose-block-delimiter fg-main)))
        :palette
        '((mode dark)
          (bg "#000000") (bg-dark "#0a0a0a") (bg-darker "#050403") (bg-alt "#111111")
          (fg "#d4c5a0") (fg-dim "#776d5c") (fg-light "#c8b890") (fg-bright "#f4e7c3")
          (accent "#d48a20") (selection "#3a2914") (muted "#756b59")
          (red "#ff5f59") (yellow "#d0bc00") (orange "#c76d52") (green "#9cbd6f")
          (cyan "#00d3d0") (blue "#5b9bd5") (magenta "#9b72cf") (brown "#8a5a32")
          (bright-red "#ff6b55") (bright-yellow "#e89e30") (bright-green "#44df44")
          (bright-cyan "#00eff0") (bright-blue "#9099d9") (bright-magenta "#b395db")
          ;; Rum's choices.
          (cursor "#9099d9")
          (bg-mode-line "#50269a") (bg-mode-line-inactive "#141414") (fg-mode-line-inactive "#756b59")
          (bg-region "#3e4451") (bg-highlight "#7030af") (bg-paren "#7030af")
          (bg-block "#2f0c3f") (bg-block-delimiter "#38325c")
          (prompt "#d2b580") (link "#4cc9f0")
          (keyword "#b395db") (function "#c28fea") (string "#aacb75") (comment "#756b59")
          (variable "#00a6a6") (constant "#72b6e4") (type "#00b8ad") (builtin "#d0a3e8")
          (fl-warning "#e89e30") (keybind "#72b6e4")
          ;; The chrome as it was mixed in sRGB: Oklab's mix toward black is darker.
          (border "#201e18") (glass "#11100d")
          (error "#c97054") (warning "#e89e30") (success "#00b8ad")))
  "Rum's theme properties for `romacs-theme-define'.")

(provide 'romacs-theme-rum)

;;; romacs-theme-rum.el ends here
