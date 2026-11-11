;;; font-family-test.el --- Test multiple font families for bold/italic spacing -*- lexical-binding: t -*-

;; Test bold/italic character spacing across different font families.
;; This exercises the layout engine's advance width computation for
;; faces that may fall back to proportional fonts when bold/italic
;; variants are unavailable.
;;
;; Usage: emacs -Q -l test/romacs/manual/font-family-test.el
;;
;; What to check:
;; - All "reference" lines should have identical character spacing
;; - Bold/italic/verbatim text should align with normal text
;; - Characters within words should not overlap or have gaps
;; - The ruler line (0123456789...) should help identify misalignment

;;; Code:

(defvar font-family-test-families
  '("Hack" "DejaVu Sans Mono" "Cascadia Mono" "JetBrains Mono"
    "Liberation Mono" "FreeMono" "Noto Sans Mono" "Fira Code"
    "Cascadia Code" "Cascadia Code NF")
  "Monospace font families to test.")

(defvar font-family-test-reference-text
  "this is a test main abcdefghijklmnopqrstuvwxyz 0123456789"
  "Reference text used for spacing comparison.")

(defun font-family-test-insert-section (family)
  "Insert a test section for FAMILY."
  ;; Try to use this font
  (condition-case err
      (progn
        ;; Create faces for this section
        (let ((bold-face (make-face (intern (format "test-bold-%s" (downcase (replace-regexp-in-string " " "-" family))))))
              (italic-face (make-face (intern (format "test-italic-%s" (downcase (replace-regexp-in-string " " "-" family))))))
              (bi-face (make-face (intern (format "test-bold-italic-%s" (downcase (replace-regexp-in-string " " "-" family)))))))
          (set-face-attribute bold-face nil :family family :weight 'bold :height 130)
          (set-face-attribute italic-face nil :family family :slant 'italic :height 130)
          (set-face-attribute bi-face nil :family family :weight 'bold :slant 'italic :height 130)

          (insert (propertize (format "=== %s ===\n" family)
                              'face '(:weight bold :height 1.2 :foreground "cyan")))
          ;; Ruler
          (insert (propertize "0         1         2         3         4         5\n"
                              'face `(:family ,family :height 130)))
          (insert (propertize "0123456789012345678901234567890123456789012345678901234567\n"
                              'face `(:family ,family :height 130)))
          ;; Normal
          (insert (propertize (format "normal:  %s\n" font-family-test-reference-text)
                              'face `(:family ,family :height 130)))
          ;; Bold
          (insert (propertize (format "bold:    %s\n" font-family-test-reference-text)
                              'face bold-face))
          ;; Italic
          (insert (propertize (format "italic:  %s\n" font-family-test-reference-text)
                              'face italic-face))
          ;; Bold + Italic
          (insert (propertize (format "bold-it: %s\n" font-family-test-reference-text)
                              'face bi-face))
          ;; Show font info
          (let* ((default-font-info (and (face-font 'default) (font-info (face-font 'default))))
                 (bold-font-obj (face-font bold-face))
                 (italic-font-obj (face-font italic-face)))
            (insert (propertize
                     (format "  [bold font: %s, italic font: %s]\n"
                             (or bold-font-obj "nil")
                             (or italic-font-obj "nil"))
                     'face '(:foreground "gray50" :height 0.8))))
          (insert "\n")))
    (error
     (insert (propertize (format "=== %s === (ERROR: %s)\n\n" family (error-message-string err))
                         'face '(:foreground "red"))))))

(defun font-family-test-insert-org-section ()
  "Insert a section testing org-mode-like markup with different faces."
  (insert (propertize "=== Org-mode style markup ===\n"
                      'face '(:weight bold :height 1.2 :foreground "yellow")))
  (insert "Normal text: " font-family-test-reference-text "\n")
  (insert (propertize "Bold text:   " 'face 'default)
          (propertize font-family-test-reference-text 'face 'bold) "\n")
  (insert (propertize "Italic text: " 'face 'default)
          (propertize font-family-test-reference-text 'face 'italic) "\n")
  (insert (propertize "Bold+Italic: " 'face 'default)
          (propertize font-family-test-reference-text 'face 'bold-italic) "\n")
  ;; Mixed inline: normal *bold* normal /italic/ normal
  (insert "Mixed: normal ")
  (insert (propertize "bold" 'face 'bold))
  (insert " normal ")
  (insert (propertize "italic" 'face 'italic))
  (insert " normal ")
  (insert (propertize "bold-italic" 'face 'bold-italic))
  (insert " normal\n")
  ;; Show resolved fonts
  (let ((bold-font (face-font 'bold))
        (italic-font (face-font 'italic)))
    (insert (propertize
             (format "  [bold: %s, italic: %s]\n\n"
                     (or bold-font "nil") (or italic-font "nil"))
             'face '(:foreground "gray50" :height 0.8)))))

(defun font-family-test-insert-proportional-fallback ()
  "Test what happens when bold face falls back to proportional font."
  (insert (propertize "=== Proportional fallback test ===\n"
                      'face '(:weight bold :height 1.2 :foreground "orange")))
  (insert "This section tests faces using explicitly proportional fonts.\n")
  (insert "Characters should still be monospace-spaced (forced by layout engine).\n\n")

  (dolist (family '("Noto Sans" "DejaVu Sans" "FreeSans"))
    (condition-case nil
        (let ((prop-face (make-face (intern (format "test-prop-%s"
                                                     (downcase (replace-regexp-in-string " " "-" family)))))))
          (set-face-attribute prop-face nil :family family :height 130)
          (insert (propertize (format "%s: " family) 'face 'default))
          (insert (propertize font-family-test-reference-text 'face prop-face))
          (insert "\n"))
      (error nil)))
  (insert "\n"))

(defun font-family-test ()
  "Run the font family test."
  (switch-to-buffer (get-buffer-create "*Font Family Test*"))
  (erase-buffer)
  (setq-local truncate-lines t)

  (insert (propertize "Font Family Spacing Test\n" 'face '(:weight bold :height 1.5 :foreground "gold")))
  (insert (propertize "========================\n" 'face '(:foreground "gold")))
  (insert "Check that bold/italic text has the same character spacing as normal text.\n")
  (insert "Any compression or stretching indicates a font metrics mismatch.\n\n")

  ;; Test default face org-mode style
  (font-family-test-insert-org-section)

  ;; Test each monospace font family
  (dolist (family font-family-test-families)
    (font-family-test-insert-section family))

  ;; Test proportional fallback
  (font-family-test-insert-proportional-fallback)

  (goto-char (point-min))
  (message "Font family test buffer created. Check bold/italic spacing alignment."))

(font-family-test)

;;; font-family-test.el ends here
