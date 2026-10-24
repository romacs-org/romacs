;;; test-fancy-box-borders.el --- Test all 10 fancy :box border styles -*- lexical-binding: t -*-

;; Load this file in romacs to display a showcase buffer with all
;; fancy rounded-box border styles.
;;
;; Usage:
;;   M-x eval-buffer  or  (load "test/romacs/test-fancy-box-borders.el")
;;
;; :box plist keys:
;;   :line-width N       — border thickness (pixels)
;;   :color COLOR        — primary border color
;;   :corner-radius N    — rounded corner radius (pixels)
;;   :border-style STYLE — symbolic style name (see below)
;;   :border-speed N     — animation speed (100 = 1.0x, 200 = 2.0x)
;;   :color2 COLOR       — secondary color (for gradient/neon)
;;
;; Styles:
;;   solid              Solid (default)
;;   rainbow            Static rainbow gradient
;;   animated-rainbow   Rotating rainbow animation
;;   gradient           Two-color gradient (:color → :color2)
;;   glow               Pulsing glow
;;   neon               Neon double-stroke
;;   dashed             Animated dashed border
;;   comet              Comet trail
;;   iridescent         Shimmering metallic
;;   fire               Fire / plasma
;;   heartbeat          Cardiac-rhythm pulsing (two-color: :color → :color2)

;; ── Define a face for each style ────────────────────────────────────

(defface fancy-box-solid
  '((t :foreground "white" :background "#1A2244"
       :box (:line-width 3 :color "dodger blue" :corner-radius 10
             :border-style solid :border-speed 100)))
  "Style: Solid rounded box.")

(defface fancy-box-rainbow
  '((t :foreground "white" :background "#222222"
       :box (:line-width 3 :color "white" :corner-radius 12
             :border-style rainbow :border-speed 100)))
  "Style: Rainbow.")

(defface fancy-box-animated-rainbow
  '((t :foreground "white" :background "#222222"
       :box (:line-width 3 :color "white" :corner-radius 12
             :border-style animated-rainbow :border-speed 100)))
  "Style: Animated Rainbow.")

(defface fancy-box-gradient
  '((t :foreground "white" :background "#331A33"
       :box (:line-width 3 :color "deep pink" :corner-radius 10
             :border-style gradient :border-speed 100 :color2 "cyan")))
  "Style: Gradient (pink → cyan).")

(defface fancy-box-glow
  '((t :foreground "white" :background "#0D330D"
       :box (:line-width 2 :color "lime green" :corner-radius 10
             :border-style glow :border-speed 100)))
  "Style: Pulsing Glow.")

(defface fancy-box-neon
  '((t :foreground "white" :background "#220D33"
       :box (:line-width 3 :color "magenta" :corner-radius 10
             :border-style neon :border-speed 100 :color2 "cyan")))
  "Style: Neon Double-Stroke.")

(defface fancy-box-dashed
  '((t :foreground "white" :background "#332D0D"
       :box (:line-width 3 :color "gold" :corner-radius 10
             :border-style dashed :border-speed 100)))
  "Style: Dashed.")

(defface fancy-box-comet
  '((t :foreground "white" :background "#331A0D"
       :box (:line-width 3 :color "orange red" :corner-radius 12
             :border-style comet :border-speed 100)))
  "Style: Comet Trail.")

(defface fancy-box-iridescent
  '((t :foreground "white" :background "#1A2233"
       :box (:line-width 3 :color "steel blue" :corner-radius 10
             :border-style iridescent :border-speed 100)))
  "Style: Iridescent.")

(defface fancy-box-fire
  '((t :foreground "white" :background "#330D0D"
       :box (:line-width 3 :color "dark red" :corner-radius 10
             :border-style fire :border-speed 100)))
  "Style: Fire / Plasma.")

(defface fancy-box-heartbeat
  '((t :foreground "white" :background "#330D0D"
       :box (:line-width 2 :color "red" :corner-radius 10
             :border-style heartbeat :border-speed 100 :color2 "green")))
  "Style: Heartbeat (red → green on beat).")

;; ── Create showcase buffer ──────────────────────────────────────────

(defun fancy-box-demo ()
  "Create a buffer showing all fancy box border styles."
  (interactive)
  (let ((buf (get-buffer-create "*Fancy Box Borders*")))
    (with-current-buffer buf
      (let ((inhibit-read-only t))
        (erase-buffer)
        (insert (propertize "Romacs Fancy Rounded Box Borders\n\n"
                            'face '(:height 1.5 :weight bold)))

        ;; Solid
        (insert "  ")
        (insert (propertize " Solid " 'face 'fancy-box-solid))
        (insert "   ")

        ;; Rainbow
        (insert (propertize " Rainbow " 'face 'fancy-box-rainbow))
        (insert "   ")

        ;; Animated Rainbow
        (insert (propertize " Animated Rainbow " 'face 'fancy-box-animated-rainbow))
        (insert "\n\n\n  ")

        ;; Gradient
        (insert (propertize " Gradient (pink->cyan) " 'face 'fancy-box-gradient))
        (insert "   ")

        ;; Pulsing Glow
        (insert (propertize " Pulsing Glow " 'face 'fancy-box-glow))
        (insert "   ")

        ;; Neon
        (insert (propertize " Neon Double-Stroke " 'face 'fancy-box-neon))
        (insert "\n\n\n  ")

        ;; Dashed
        (insert (propertize " Dashed " 'face 'fancy-box-dashed))
        (insert "   ")

        ;; Comet
        (insert (propertize " Comet Trail " 'face 'fancy-box-comet))
        (insert "   ")

        ;; Iridescent
        (insert (propertize " Iridescent " 'face 'fancy-box-iridescent))
        (insert "\n\n\n  ")

        ;; Fire
        (insert (propertize " Fire / Plasma " 'face 'fancy-box-fire))
        (insert "   ")

        ;; Heartbeat
        (insert (propertize " Heartbeat " 'face 'fancy-box-heartbeat))
        (insert "\n\n\n")

        ;; Speed variations
        (insert (propertize "Speed Variations (Animated Rainbow):\n\n"
                            'face '(:height 1.2 :weight bold)))
        (insert "  ")
        (insert (propertize " 0.5x speed "
                            'face '(:foreground "white" :background "#222222"
                                    :box (:line-width 3 :color "white" :corner-radius 10
                                          :border-style animated-rainbow :border-speed 50))))
        (insert "   ")
        (insert (propertize " 1.0x speed "
                            'face '(:foreground "white" :background "#222222"
                                    :box (:line-width 3 :color "white" :corner-radius 10
                                          :border-style animated-rainbow :border-speed 100))))
        (insert "   ")
        (insert (propertize " 2.0x speed "
                            'face '(:foreground "white" :background "#222222"
                                    :box (:line-width 3 :color "white" :corner-radius 10
                                          :border-style animated-rainbow :border-speed 200))))
        (insert "   ")
        (insert (propertize " 4.0x speed "
                            'face '(:foreground "white" :background "#222222"
                                    :box (:line-width 3 :color "white" :corner-radius 10
                                          :border-style animated-rainbow :border-speed 400))))
        (insert "\n\n\n")

        ;; Different border widths
        (insert (propertize "Border Width Variations (Heartbeat):\n\n"
                            'face '(:height 1.2 :weight bold)))
        (insert "  ")
        (insert (propertize " 1px "
                            'face '(:foreground "white" :background "#330D0D"
                                    :box (:line-width 1 :color "red" :corner-radius 8
                                          :border-style heartbeat :border-speed 100 :color2 "green"))))
        (insert "   ")
        (insert (propertize " 2px "
                            'face '(:foreground "white" :background "#330D0D"
                                    :box (:line-width 2 :color "red" :corner-radius 10
                                          :border-style heartbeat :border-speed 100 :color2 "green"))))
        (insert "   ")
        (insert (propertize " 4px "
                            'face '(:foreground "white" :background "#330D0D"
                                    :box (:line-width 4 :color "red" :corner-radius 12
                                          :border-style heartbeat :border-speed 100 :color2 "green"))))
        (insert "\n\n\n")

        ;; Color combinations
        (insert (propertize "Gradient Color Combos:\n\n"
                            'face '(:height 1.2 :weight bold)))
        (insert "  ")
        (insert (propertize " Sunset "
                            'face '(:foreground "white" :background "#331A0D"
                                    :box (:line-width 3 :color "red" :corner-radius 10
                                          :border-style gradient :border-speed 100 :color2 "gold"))))
        (insert "   ")
        (insert (propertize " Ocean "
                            'face '(:foreground "white" :background "#0D1A33"
                                    :box (:line-width 3 :color "navy" :corner-radius 10
                                          :border-style gradient :border-speed 100 :color2 "aquamarine"))))
        (insert "   ")
        (insert (propertize " Forest "
                            'face '(:foreground "white" :background "#0D330D"
                                    :box (:line-width 3 :color "dark green" :corner-radius 10
                                          :border-style gradient :border-speed 100 :color2 "lime green"))))
        (insert "   ")
        (insert (propertize " Galaxy "
                            'face '(:foreground "white" :background "#220D22"
                                    :box (:line-width 3 :color "purple" :corner-radius 10
                                          :border-style gradient :border-speed 100 :color2 "hot pink"))))
        (insert "\n"))
      (goto-char (point-min))
      (setq buffer-read-only t))
    (switch-to-buffer buf)))

;; Run the demo
(fancy-box-demo)

(provide 'test-fancy-box-borders)
;;; test-fancy-box-borders.el ends here
