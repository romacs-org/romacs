;;; rum-theme.el --- Warm accents on black, derived from Modus -*- lexical-binding: t -*-

;; Copyright (C) 2026 Thanos Apollo
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Rum builds on the bundled Modus Vivendi palette.

;;; Code:

(require-theme 'modus-themes)

(defvar rum-palette nil)
(defvar rum-palette-user nil)
(defvar rum-palette-overrides nil)

(setq rum-palette
      (modus-themes-generate-palette
       '((bg-main "#000000")
         (fg-main "#d4c5a0")
         (fg-dim "#756b59")

         (red "#c97054")
         (red-warmer "#ff6b55")
         (red-cooler "#d8796f")
         (red-faint "#c98f7d")
         (red-intense "#ff5f59")

         (green "#9cbd6f")
         (green-warmer "#b3a35c")
         (green-cooler "#78a68b")
         (green-faint "#a6bd95")
         (green-intense "#44df44")

         (yellow "#d48a20")
         (yellow-warmer "#e89e30")
         (yellow-cooler "#c8b890")
         (yellow-faint "#d2b580")
         (yellow-intense "#f0b84b")
         (gold "#d48a20")
         (fg-link-rum "#4cc9f0")

         (blue "#5b9bd5")
         (blue-warmer "#9099d9")
         (blue-cooler "#72b6e4")
         (blue-faint "#7ea8bd")
         (blue-intense "#72b6e4")
         (indigo "#9099d9")

         (magenta "#c28fea")
         (magenta-warmer "#d0a3e8")
         (magenta-cooler "#b395db")
         (magenta-faint "#aa91c2")
         (magenta-intense "#c28fea")

         (cyan "#00a6a6")
         (cyan-warmer "#4fb8b4")
         (cyan-cooler "#00b8ad")
         (cyan-faint "#8bbcb2")
         (cyan-intense "#00d3d0")

         (olive "#aacb75")
         (bg-lavender "#38325c")
         (bg-mode-line-rum "#50269a")
         (bg-magenta-intense "#7030af")
         (bg-magenta-nuanced "#2f0c3f")
         (bg-term-white "#a6a6a6"))
       'warm
       modus-themes-vivendi-palette))

(setq rum-palette-overrides
      '((comment fg-dim)
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

(let ((modus-themes-italic-constructs nil)
      (modus-themes-bold-constructs nil)
      (modus-themes-mixed-fonts nil)
      (modus-themes-variable-pitch-ui nil)
      (modus-themes-prompts '(semibold italic))
      (modus-themes-completions '((matches . (bold))
                                 (selection . (semibold text-also))))
      (modus-themes-headings '((1 . (ultrabold 1.35))
                              (2 . (semibold 1.2))
                              (agenda-date . (1.2))
                              (agenda-structure . (variable-pitch light 1.8))
                              (t . (1.15)))))
  (modus-themes-theme
   'rum
   'modus-themes
   "Warm parchment and restrained accents on a black background."
   'dark
   'rum-palette
   'rum-palette-user
   'rum-palette-overrides))

(let ((bg (cadr (assq 'bg-main rum-palette)))
      (gold (cadr (assq 'gold rum-palette))))
  (custom-theme-set-faces
   'rum
   ;; Romacs's chrome accent (romacs-chrome.el): the focus edge, a layer's
   ;; glow and completion matches.
   `(romacs-layer-glow ((t (:foreground ,gold))))
   `(fringe ((t (:background ,bg :foreground ,bg))))
   `(window-divider ((t (:background ,bg :foreground ,bg))))
   `(window-divider-first-pixel ((t (:background ,bg :foreground ,bg))))
   `(window-divider-last-pixel ((t (:background ,bg :foreground ,bg))))))

;;; rum-theme.el ends here
