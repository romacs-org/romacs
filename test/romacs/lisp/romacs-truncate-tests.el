;;; romacs-truncate-tests.el --- Text kept within its width -*- lexical-binding: t; -*-
(require 'ert)
(require 'romacs-truncate)
(require 'romacs-attach)
(require 'romacs-layer)
(require 'hermes-chat-buffer)

(defconst romacs-truncate-tests--long "abcdefghijklmnop.txt")

(ert-deftest romacs-truncate-never-passes-its-width ()
  (let ((truncate-string-ellipsis "..."))
    (dotimes (width 12)
      (should (<= (string-width (romacs-truncate romacs-truncate-tests--long width)) width)))))

(ert-deftest romacs-truncate-keeps-the-ellipsis-only-with-room-for-text ()
  (let ((truncate-string-ellipsis "..."))
    (should (equal (romacs-truncate "abcdefgh" 2) "ab"))
    (should (equal (romacs-truncate "abcdefgh" 3) "abc"))
    (should (equal (romacs-truncate "abcdefgh" 4) "a..."))
    (should (equal (romacs-truncate "abc" 8) "abc"))
    (should (equal (romacs-truncate "abc" -1) ""))
    (should (equal (romacs-truncate "abc" 5 ?\s) "abc  "))))

(ert-deftest romacs-truncate-a-chip-s-name-fits-its-width ()
  (let ((truncate-string-ellipsis "..."))
    (dotimes (width 24)
      (should (<= (string-width (romacs-attach--elide "abcdef.txt" width)) width))
      (should (<= (string-width (romacs-attach--elide romacs-truncate-tests--long width)) width)))
    (should (equal (romacs-attach--elide romacs-truncate-tests--long 14) "abcd...nop.txt"))))

(ert-deftest romacs-truncate-a-left-cut-keeps-what-fits ()
  (let ((truncate-string-ellipsis "..."))
    (should (equal (hermes-chat--truncate-left "/a/b/c/d" 2) "/d"))
    (should (equal (hermes-chat--truncate-left "/a/b/c/d" 5) ".../d"))
    (dotimes (width 10)
      (should (<= (string-width (hermes-chat--truncate-left "/a/b/c/d/e/f" width)) width)))))

(ert-deftest romacs-truncate-a-toast-stays-within-its-columns ()
  (let ((truncate-string-ellipsis "...") (romacs-layer-toast-lines 1))
    (dolist (columns '(2 4 5 8 12 30))
      (dolist (row (split-string (romacs-layer--toast-text "Wrote /long/file.txt\nsecond\nthird" columns) "\n"))
        (should (<= (string-width row) columns))))))

(provide 'romacs-truncate-tests)
;;; romacs-truncate-tests.el ends here
