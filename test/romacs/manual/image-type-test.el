;;; image-type-test.el --- Test all image types via Rust GPU pipeline -*- lexical-binding: t -*-

;; Tests PNG, JPEG, GIF, TIFF, WebP, SVG file, SVG inline data,
;; auto-detect, image-size, and scaling.  All decoding goes through
;; the Rust image/resvg crates — no C image libraries needed.

(defvar image-type-test-dir (expand-file-name "~/Pictures/"))

(sit-for 2)

(switch-to-buffer (get-buffer-create "*Image Type Test*"))
(erase-buffer)
(setq truncate-lines t)

(insert "=== Romacs GPU Image Rendering Test ===\n\n")

;; ---- image-type-available-p ------------------------------------------------
(insert "image-type-available-p:\n")
(dolist (type '(png jpeg gif tiff webp svg romacs))
  (insert (format "  %s: %s  " type (if (image-type-available-p type) "YES" "no"))))
(insert "\n\n")

;; ---- Row 1: PNG, JPEG, GIF ------------------------------------------------
(insert "--- PNG --- ")
(condition-case err
    (insert-image (create-image (expand-file-name "test.png" image-type-test-dir)
                                'png nil :max-width 100 :max-height 80)
                  "[PNG]")
  (error (insert (format "ERR:%S" err))))

(insert "  --- JPEG --- ")
(condition-case err
    (insert-image (create-image (expand-file-name "test.jpg" image-type-test-dir)
                                'jpeg nil :max-width 100 :max-height 80)
                  "[JPEG]")
  (error (insert (format "ERR:%S" err))))

(insert "  --- GIF --- ")
(condition-case err
    (insert-image (create-image (expand-file-name "test.gif" image-type-test-dir)
                                'gif nil :max-width 100 :max-height 80)
                  "[GIF]")
  (error (insert (format "ERR:%S" err))))
(insert "\n\n")

;; ---- Row 2: TIFF, WebP, SVG file ------------------------------------------
(insert "--- TIFF --- ")
(condition-case err
    (insert-image (create-image (expand-file-name "test.tiff" image-type-test-dir)
                                'tiff nil :max-width 100 :max-height 80)
                  "[TIFF]")
  (error (insert (format "ERR:%S" err))))

(insert "  --- WebP --- ")
(condition-case err
    (insert-image (create-image (expand-file-name "test.webp" image-type-test-dir)
                                'webp nil :max-width 100 :max-height 80)
                  "[WebP]")
  (error (insert (format "ERR:%S" err))))

(insert "  --- SVG file --- ")
(condition-case err
    (insert-image (create-image (expand-file-name "test.svg" image-type-test-dir)
                                'svg nil :max-width 80 :max-height 80)
                  "[SVG]")
  (error (insert (format "ERR:%S" err))))
(insert "\n\n")

;; ---- Row 3: SVG inline data (shapes) --------------------------------------
(insert "--- SVG data (shapes) --- ")
(condition-case err
    (let ((svg "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"80\"><rect width=\"120\" height=\"80\" fill=\"#f0e68c\"/><circle cx=\"40\" cy=\"40\" r=\"25\" fill=\"#ff6347\"/><rect x=\"70\" y=\"15\" width=\"40\" height=\"50\" rx=\"4\" fill=\"#4169e1\"/></svg>"))
      (insert-image (create-image svg 'svg t) "[SVG-shapes]"))
  (error (insert (format "ERR:%S" err))))
(insert "\n\n")

;; ---- Row 4: SVG inline data (text) ----------------------------------------
(insert "--- SVG data (text) --- ")
(condition-case err
    (let ((svg "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"60\"><rect width=\"200\" height=\"60\" fill=\"#e8e8e8\"/><text x=\"10\" y=\"40\" font-family=\"sans-serif\" font-size=\"24\" fill=\"#333\">Hello SVG!</text></svg>"))
      (insert-image (create-image svg 'svg t) "[SVG-text]"))
  (error (insert (format "ERR:%S" err))))
(insert "\n\n")

;; ---- Row 5: Auto-detect (nil type) ----------------------------------------
(insert "--- Auto-detect ---  ")
(condition-case err
    (insert-image (create-image (expand-file-name "test.png" image-type-test-dir)
                                nil nil :max-width 50 :max-height 50)
                  "[a-png]")
  (error (insert (format "ERR" err))))
(insert " ")
(condition-case err
    (insert-image (create-image (expand-file-name "test.jpg" image-type-test-dir)
                                nil nil :max-width 50 :max-height 50)
                  "[a-jpg]")
  (error (insert (format "ERR" err))))
(insert " ")
(condition-case err
    (insert-image (create-image (expand-file-name "test.webp" image-type-test-dir)
                                nil nil :max-width 50 :max-height 50)
                  "[a-webp]")
  (error (insert (format "ERR" err))))
(insert "\n\n")

;; ---- image-size ------------------------------------------------------------
(insert "--- image-size ---\n")
(dolist (spec `(("test.png" png)
                ("test.jpg" jpeg)
                ("test.svg" svg)))
  (let ((file (expand-file-name (car spec) image-type-test-dir))
        (type (cadr spec)))
    (condition-case err
        (let* ((img (create-image file type))
               (size (image-size img t)))
          (insert (format "  %s: %sx%s  " type (car size) (cdr size))))
      (error (insert (format "  %s: ERR  " type))))))

(insert "\n\n=== ALL DONE ===\n")
(goto-char (point-min))
(redisplay t)
(message "Waiting 8s for async images...")
(sit-for 8)
(redisplay t)
(message "Test complete")
