;;; bidi-test.el --- Test bidirectional text rendering -*- lexical-binding: t -*-

;; Test bidirectional (bidi) text rendering in the romacs GPU renderer.
;; Requires a font with Hebrew and Arabic glyph support (e.g., Noto Sans,
;; DejaVu Sans, or any font covering Unicode blocks U+0590-U+05FF and
;; U+0600-U+06FF).  If RTL text appears as boxes or tofu, install an
;; appropriate font and restart.
;; Usage: ./target/release/romacs -Q -l test/romacs/bidi-test.el

;;; Code:

(defun bidi-test--insert-section (title description)
  "Insert a section header with TITLE and DESCRIPTION explaining expected output."
  (insert "\n")
  (let ((start (point)))
    (insert (format "=== %s ===\n" title))
    (put-text-property start (point) 'face '(:weight bold :foreground "gold" :height 1.2)))
  (let ((start (point)))
    (insert (format "  [Expected: %s]\n\n" description))
    (put-text-property start (point) 'face '(:foreground "dim gray" :slant italic))))

(defun bidi-test--insert-labeled (label text &optional face)
  "Insert LABEL followed by TEXT with optional FACE."
  (insert (format "  %-36s" label))
  (if face
      (let ((start (point)))
        (insert text)
        (put-text-property start (point) 'face face))
    (insert text))
  (insert "\n"))

(defun bidi-test ()
  "Create buffer exercising bidirectional text rendering."
  (interactive)
  (let ((buf (get-buffer-create "*Bidi Test*")))
    (switch-to-buffer buf)
    (erase-buffer)

    ;; Title
    (let ((start (point)))
      (insert "BIDIRECTIONAL TEXT RENDERING TEST\n")
      (put-text-property start (point) 'face '(:weight bold :height 1.8 :foreground "cyan")))
    (insert (format "Window system: %s\n" window-system))
    (insert (format "Default font: %s\n" (face-attribute 'default :family)))
    (insert (make-string 70 ?-) "\n")
    (insert "\n")
    (insert "NOTE: This test requires a font with Hebrew (U+0590-U+05FF) and\n")
    (insert "Arabic (U+0600-U+06FF) support.  If you see empty boxes or tofu,\n")
    (insert "install Noto Sans Hebrew / Noto Sans Arabic and restart.\n")

    ;; ========================================================================
    ;; Test 1: Pure RTL text
    ;; ========================================================================
    (bidi-test--insert-section
     "1. Pure RTL Text"
     "Hebrew and Arabic text should flow right-to-left")

    (bidi-test--insert-labeled "Hebrew (shalom olam):" "שלום עולם")
    (bidi-test--insert-labeled "Arabic (marhaba balalaam):" "مرحبا بالعالم")
    (bidi-test--insert-labeled "Hebrew sentence:" "זוהי שורה בעברית לבדיקה")
    (bidi-test--insert-labeled "Arabic sentence:" "هذا سطر باللغة العربية للاختبار")
    (bidi-test--insert-labeled "Hebrew multi-word:" "אחד שתיים שלוש ארבע חמש")
    (bidi-test--insert-labeled "Arabic multi-word:" "واحد اثنان ثلاثة أربعة خمسة")

    ;; ========================================================================
    ;; Test 2: Pure LTR text (baseline)
    ;; ========================================================================
    (bidi-test--insert-section
     "2. Pure LTR Text (Baseline)"
     "Standard left-to-right English text for comparison")

    (bidi-test--insert-labeled "English:" "Hello World")
    (bidi-test--insert-labeled "English sentence:" "The quick brown fox jumps over the lazy dog.")
    (bidi-test--insert-labeled "ASCII symbols:" "!@#$%^&*()_+-=[]{}|;':\",./<>?")

    ;; ========================================================================
    ;; Test 3: Mixed LTR + RTL on the same line
    ;; ========================================================================
    (bidi-test--insert-section
     "3. Mixed LTR + RTL on Same Line"
     "English and Hebrew/Arabic interspersed; each run should keep its direction")

    (bidi-test--insert-labeled "EN-HE-EN:" "Hello שלום World")
    (bidi-test--insert-labeled "EN-AR-EN:" "Hello مرحبا World")
    (bidi-test--insert-labeled "HE-EN-HE:" "שלום Hello עולם")
    (bidi-test--insert-labeled "AR-EN-AR:" "مرحبا Hello بالعالم")
    (bidi-test--insert-labeled "Multiple switches:" "Start שלום middle مرحبا end")
    (bidi-test--insert-labeled "Nested embedding:" "The word שלום means peace")
    (bidi-test--insert-labeled "EN prefix + HE suffix:" "Version: גרסה חדשה")
    (bidi-test--insert-labeled "HE prefix + EN suffix:" "גרסה: version 2.0")

    ;; ========================================================================
    ;; Test 4: Numbers embedded in RTL text
    ;; ========================================================================
    (bidi-test--insert-section
     "4. Numbers in RTL Context"
     "Numbers (European digits) should remain LTR even inside RTL text")

    (bidi-test--insert-labeled "HE + numbers:" "מחיר: 42 שקלים")
    (bidi-test--insert-labeled "AR + numbers:" "السعر: 42 ريال")
    (bidi-test--insert-labeled "HE phone number:" "טלפון: 03-1234567")
    (bidi-test--insert-labeled "AR date:" "التاريخ: 2026-02-11")
    (bidi-test--insert-labeled "HE math:" "התוצאה היא 3.14 בקירוב")
    (bidi-test--insert-labeled "AR percentage:" "النسبة: 95.5%")
    (bidi-test--insert-labeled "HE mixed nums:" "בין 100 ל-200 פריטים")
    (bidi-test--insert-labeled "AR version:" "الإصدار 3.2.1 متاح")

    ;; ========================================================================
    ;; Test 5: Parentheses and bracket mirroring
    ;; ========================================================================
    (bidi-test--insert-section
     "5. Parentheses and Bracket Mirroring"
     "Brackets should mirror in RTL context: ( becomes ) visually, etc.")

    (bidi-test--insert-labeled "HE parens:" "שלום (עולם) טוב")
    (bidi-test--insert-labeled "AR parens:" "مرحبا (بالعالم) جميل")
    (bidi-test--insert-labeled "HE brackets:" "ערך [ראשון] ואחרון")
    (bidi-test--insert-labeled "AR brackets:" "قيمة [أولى] وأخيرة")
    (bidi-test--insert-labeled "HE braces:" "קוד {בלוק} כאן")
    (bidi-test--insert-labeled "AR braces:" "كود {بلوك} هنا")
    (bidi-test--insert-labeled "HE angle brackets:" "תגית <שם> סוף")
    (bidi-test--insert-labeled "AR angle brackets:" "وسم <اسم> نهاية")
    (bidi-test--insert-labeled "Nested HE:" "חיצוני (פנימי [עמוק]) סוף")
    (bidi-test--insert-labeled "Mixed EN(HE)EN:" "open (שלום) close")

    ;; ========================================================================
    ;; Test 6: Bidi text with face attributes
    ;; ========================================================================
    (bidi-test--insert-section
     "6. Bidirectional Text with Face Attributes"
     "RTL text with bold, italic, color, underline should render correctly")

    (insert "  Bold Hebrew:                      ")
    (let ((start (point)))
      (insert "שלום עולם")
      (put-text-property start (point) 'face '(:weight bold)))
    (insert "\n")

    (insert "  Italic Arabic:                    ")
    (let ((start (point)))
      (insert "مرحبا بالعالم")
      (put-text-property start (point) 'face '(:slant italic)))
    (insert "\n")

    (insert "  Red Hebrew:                       ")
    (let ((start (point)))
      (insert "שלום עולם")
      (put-text-property start (point) 'face '(:foreground "red")))
    (insert "\n")

    (insert "  Green Arabic:                     ")
    (let ((start (point)))
      (insert "مرحبا بالعالم")
      (put-text-property start (point) 'face '(:foreground "green")))
    (insert "\n")

    (insert "  BG highlight Hebrew:              ")
    (let ((start (point)))
      (insert "שלום עולם")
      (put-text-property start (point) 'face '(:background "dark red" :foreground "white")))
    (insert "\n")

    (insert "  Underlined Arabic:                ")
    (let ((start (point)))
      (insert "مرحبا بالعالم")
      (put-text-property start (point) 'face '(:underline t)))
    (insert "\n")

    (insert "  Bold+Color Hebrew:                ")
    (let ((start (point)))
      (insert "שלום עולם")
      (put-text-property start (point) 'face '(:weight bold :foreground "cyan")))
    (insert "\n")

    (insert "  Large Arabic:                     ")
    (let ((start (point)))
      (insert "مرحبا بالعالم")
      (put-text-property start (point) 'face '(:height 1.5 :foreground "gold")))
    (insert "\n")

    (insert "  Box face Hebrew:                  ")
    (let ((start (point)))
      (insert "שלום עולם")
      (put-text-property start (point) 'face '(:box (:line-width 1 :color "orange"))))
    (insert "\n")

    (insert "  Mixed face run:                   ")
    (let ((s1 (point)))
      (insert "Hello ")
      (put-text-property s1 (point) 'face '(:foreground "yellow")))
    (let ((s2 (point)))
      (insert "שלום")
      (put-text-property s2 (point) 'face '(:foreground "cyan" :weight bold)))
    (insert " ")
    (let ((s3 (point)))
      (insert "مرحبا")
      (put-text-property s3 (point) 'face '(:foreground "magenta" :slant italic)))
    (let ((s4 (point)))
      (insert " World")
      (put-text-property s4 (point) 'face '(:foreground "yellow")))
    (insert "\n")

    ;; ========================================================================
    ;; Test 7: RTL text with line numbers
    ;; ========================================================================
    (bidi-test--insert-section
     "7. RTL Text with Line Numbers"
     "Enable display-line-numbers-mode; numbers should stay on the left margin")

    (insert "  To test: M-x display-line-numbers-mode RET in this buffer.\n")
    (insert "  Line numbers should remain on the left side for all lines below.\n\n")
    (insert "  שורה ראשונה בעברית\n")
    (insert "  שורה שנייה בעברית\n")
    (insert "  שורה שלישית בעברית\n")
    (insert "  سطر أول بالعربية\n")
    (insert "  سطر ثاني بالعربية\n")
    (insert "  سطر ثالث بالعربية\n")
    (insert "  English line between RTL lines\n")
    (insert "  שורה אחרי אנגלית\n")
    (insert "  سطر بعد الإنجليزية\n")

    ;; ========================================================================
    ;; Test 8: RTL text with word-wrap
    ;; ========================================================================
    (bidi-test--insert-section
     "8. RTL Text with Word Wrap"
     "Long RTL lines should wrap at word boundaries when visual-line-mode is on")

    (insert "  To test: M-x visual-line-mode RET then resize the window narrower.\n\n")
    ;; Long Hebrew paragraph
    (insert "  ")
    (insert "זוהי פסקה ארוכה בעברית שנועדה לבדוק את מנגנון גלישת המילים במנוע הרינדור של ניאומקס. כאשר חלון העריכה צר מספיק, הטקסט צריך לעבור לשורה הבאה בגבולות מילים ולא באמצע מילה. המילים צריכות להישאר שלמות וסדר הקריאה צריך להיות מימין לשמאל גם אחרי הגלישה.")
    (insert "\n\n")
    ;; Long Arabic paragraph
    (insert "  ")
    (insert "هذه فقرة طويلة باللغة العربية مصممة لاختبار آلية التفاف الكلمات في محرك العرض. عندما تكون نافذة التحرير ضيقة بما فيه الكفاية، يجب أن ينتقل النص إلى السطر التالي عند حدود الكلمات وليس في منتصف الكلمة. يجب أن تبقى الكلمات كاملة ويجب أن يكون اتجاه القراءة من اليمين إلى اليسار حتى بعد الالتفاف.")
    (insert "\n\n")
    ;; Long mixed paragraph
    (insert "  ")
    (insert "This is a mixed paragraph with שלום עולם Hebrew words and also مرحبا بالعالم Arabic words interspersed throughout the text to test how the word-wrap engine handles direction changes at wrap boundaries. The renderer should keep each bidi run intact when breaking lines.")
    (insert "\n")

    ;; ========================================================================
    ;; Test 9: Multiple paragraphs alternating LTR and RTL
    ;; ========================================================================
    (bidi-test--insert-section
     "9. Alternating LTR/RTL Paragraphs"
     "Consecutive paragraphs alternating direction; each should use its own base direction")

    (insert "  This is an English paragraph.  It flows left to right.\n")
    (insert "  The text alignment should be on the left side.\n")
    (insert "\n")
    (insert "  זוהי פסקה בעברית. היא זורמת מימין לשמאל.\n")
    (insert "  יישור הטקסט צריך להיות בצד ימין.\n")
    (insert "\n")
    (insert "  Back to English again.  This paragraph is LTR.\n")
    (insert "  Everything should read naturally left to right.\n")
    (insert "\n")
    (insert "  هذه فقرة بالعربية. تتدفق من اليمين إلى اليسار.\n")
    (insert "  محاذاة النص يجب أن تكون على الجانب الأيمن.\n")
    (insert "\n")
    (insert "  Final English paragraph to end the sequence.\n")

    ;; ========================================================================
    ;; Test 10: RTL text with overlays
    ;; ========================================================================
    (bidi-test--insert-section
     "10. RTL Text with Overlays"
     "Overlays should apply correctly to RTL text without breaking bidi reordering")

    ;; Highlight overlay on Hebrew
    (insert "  Highlight overlay on Hebrew:      ")
    (let ((start (point)))
      (insert "שלום עולם טוב")
      (let ((ov (make-overlay start (point))))
        (overlay-put ov 'face '(:background "dark green" :foreground "white"))))
    (insert "\n")

    ;; Highlight overlay on Arabic
    (insert "  Highlight overlay on Arabic:      ")
    (let ((start (point)))
      (insert "مرحبا بالعالم الجميل")
      (let ((ov (make-overlay start (point))))
        (overlay-put ov 'face '(:background "dark blue" :foreground "white"))))
    (insert "\n")

    ;; Partial overlay on Hebrew word
    (insert "  Partial overlay (mid-word HE):    ")
    (let ((start (point)))
      (insert "שלום עולם טוב")
      ;; Overlay just "עולם" (bytes 11-19 in the string, chars 5-9)
      (let ((ov (make-overlay (+ start 5) (+ start 9))))
        (overlay-put ov 'face '(:background "dark red" :weight bold))))
    (insert "\n")

    ;; Overlay with before/after-string on RTL
    (insert "  Before/after-string overlay HE:   ")
    (let ((start (point)))
      (insert "שלום עולם")
      (let ((ov (make-overlay start (point))))
        (overlay-put ov 'before-string
                     (propertize "[START]" 'face '(:foreground "lime green")))
        (overlay-put ov 'after-string
                     (propertize "[END]" 'face '(:foreground "orange red")))))
    (insert "\n")

    ;; Overlay with before/after-string on Arabic
    (insert "  Before/after-string overlay AR:   ")
    (let ((start (point)))
      (insert "مرحبا بالعالم")
      (let ((ov (make-overlay start (point))))
        (overlay-put ov 'before-string
                     (propertize "[START]" 'face '(:foreground "lime green")))
        (overlay-put ov 'after-string
                     (propertize "[END]" 'face '(:foreground "orange red")))))
    (insert "\n")

    ;; Underline overlay on mixed bidi line
    (insert "  Underline overlay on mixed line:  ")
    (let ((start (point)))
      (insert "Hello שלום مرحبا World")
      (let ((ov (make-overlay start (point))))
        (overlay-put ov 'face '(:underline (:style wave :color "yellow")))))
    (insert "\n")

    ;; Inverse-video overlay on RTL
    (insert "  Inverse-video overlay on HE:      ")
    (let ((start (point)))
      (insert "שלום עולם")
      (let ((ov (make-overlay start (point))))
        (overlay-put ov 'face '(:inverse-video t))))
    (insert "\n")

    ;; Multiple overlapping overlays on RTL
    (insert "  Overlapping overlays on HE:       ")
    (let ((start (point)))
      (insert "אחד שתיים שלוש")
      ;; First overlay: background on "אחד שתיים"
      (let ((ov1 (make-overlay start (+ start 9))))
        (overlay-put ov1 'face '(:background "midnight blue")))
      ;; Second overlay: bold on "שתיים שלוש"
      (let ((ov2 (make-overlay (+ start 4) (point))))
        (overlay-put ov2 'face '(:weight bold :foreground "cyan"))))
    (insert "\n")

    ;; ========================================================================
    ;; Footer
    ;; ========================================================================
    (insert "\n")
    (let ((start (point)))
      (insert (make-string 70 ?-) "\n")
      (put-text-property start (point) 'face '(:foreground "dim gray")))
    (insert "\n")
    (insert "Keybindings for interactive testing:\n")
    (insert "  L   - Toggle display-line-numbers-mode (for test 7)\n")
    (insert "  W   - Toggle visual-line-mode (for test 8)\n")
    (insert "  D   - Toggle bidi-display-reordering\n")
    (insert "  P   - Set bidi-paragraph-direction to nil (auto-detect)\n")
    (insert "  R   - Set bidi-paragraph-direction to right-to-left\n")
    (insert "  E   - Set bidi-paragraph-direction to left-to-right\n")
    (insert "  q   - Kill test buffer\n")

    ;; Keybindings
    (local-set-key (kbd "L") (lambda () (interactive)
                               (display-line-numbers-mode 'toggle)
                               (message "display-line-numbers-mode: %s"
                                        (if display-line-numbers-mode "ON" "OFF"))))
    (local-set-key (kbd "W") (lambda () (interactive)
                               (visual-line-mode 'toggle)
                               (message "visual-line-mode: %s"
                                        (if visual-line-mode "ON" "OFF"))))
    (local-set-key (kbd "D") (lambda () (interactive)
                               (setq bidi-display-reordering
                                     (not bidi-display-reordering))
                               (force-window-update)
                               (message "bidi-display-reordering: %s"
                                        bidi-display-reordering)))
    (local-set-key (kbd "P") (lambda () (interactive)
                               (setq bidi-paragraph-direction nil)
                               (force-window-update)
                               (message "bidi-paragraph-direction: nil (auto-detect)")))
    (local-set-key (kbd "R") (lambda () (interactive)
                               (setq bidi-paragraph-direction 'right-to-left)
                               (force-window-update)
                               (message "bidi-paragraph-direction: right-to-left")))
    (local-set-key (kbd "E") (lambda () (interactive)
                               (setq bidi-paragraph-direction 'left-to-right)
                               (force-window-update)
                               (message "bidi-paragraph-direction: left-to-right")))
    (local-set-key (kbd "q") (lambda () (interactive)
                               (kill-buffer (current-buffer))
                               (message "Bidi test buffer killed.")))

    (goto-char (point-min))
    (setq buffer-read-only t)
    (message "Bidi test ready. Press L/W/D/P/R/E for interactive toggles, q to quit.")))

(bidi-test)

;;; bidi-test.el ends here
