;;; cursor-height-test.el --- Reproduce cursor bug with mixed-height faces -*- lexical-binding: t -*-

;; Reproduce: cursor on 'aa' (height 1.0) vs 'bb' (height 2.0) on same line.
;; Usage: ./target/release/romacs -Q -l test/romacs/manual/cursor-height-test.el

;;; Code:

(delete-other-windows)
(switch-to-buffer (get-buffer-create "*cursor-height-bug*"))
(erase-buffer)

;; Insert "aabb" where aa is height 1.0 and bb is height 2.0
;; Use default fg/bg (no explicit :foreground/:background)
(let ((s (point)))
  (insert "aa")
  (put-text-property s (point) 'face '(:height 1.0)))
(let ((s (point)))
  (insert "bb")
  (put-text-property s (point) 'face '(:height 2.0)))

(insert "\n")
(insert "\nMove cursor between 'aa' (height 1.0) and 'bb' (height 2.0).\n")
(insert "Cursor should be a filled box matching the character height.\n")

;; Place cursor on the first 'a'
(goto-char (point-min))

(message "Cursor on 'a' (height 1.0). Press C-f to move to 'b' (height 2.0).")

;;; cursor-height-test.el ends here
